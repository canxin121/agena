# Agent profiles

An agent profile is a named, reusable set of instructions, permission defaults,
model preferences, and turn limits for a session. A server can run a development
session and a review session with different profiles at the same time. The
selected name is saved with the session and survives reconnects and restarts.

Profiles use Agena's existing prompt layers, permission configuration, and model
selection. They are defined under the top-level `profiles` object. The separate
`plugins.profiles` setting continues to configure plugin deployment.

## Define profiles

Put shared profiles in the instance configuration, normally
`~/agena/agena.json` (or `<data-dir>/agena.json` when using `--data-dir`). Put
project profiles in `<workspace>/.agena/agena.json`.

This example defines a development profile and a review profile that inherits
its settings. The profile names are configuration keys; titles and descriptions
are display metadata.

```json
{
  "default_profile": "developer",
  "profiles": {
    "developer": {
      "title": "Developer",
      "description": "Implement and verify changes in this workspace.",
      "prompts": {
        "order": ["development-workflow"],
        "inline": [
          {
            "id": "development-workflow",
            "title": "Development workflow",
            "body": "Inspect the existing implementation, make focused changes, and verify the resulting behavior."
          }
        ]
      },
      "permission": {
        "path": {
          "workspace": "readwrite",
          "external": "deny"
        }
      },
      "max_turns": 100
    },
    "reviewer": {
      "extends": "developer",
      "title": "Reviewer",
      "description": "Review correctness, regressions, and missing tests.",
      "prompts": {
        "mode": "replace",
        "order": ["review-workflow"],
        "inline": [
          {
            "id": "review-workflow",
            "title": "Review workflow",
            "body": "Review correctness, regressions, and missing tests. Report actionable findings in severity order with file references and explanations."
          }
        ]
      },
      "permission": {
        "path": {
          "workspace": "readonly",
          "external": "deny"
        }
      },
      "max_turns": 40
    }
  }
}
```

Every field inside a profile is optional. Omitting `default_profile` leaves
existing startup behavior in place unless a broader configuration supplies a
default. Setting `"default_profile": ""` in the project configuration explicitly
disables an inherited default. `null` or an omitted field inherits it.

| Profile field | Meaning |
| --- | --- |
| `title` | Human-readable name in profile pickers. |
| `description` | Explanation shown alongside the name. |
| `extends` | Name of one parent profile. Parents are resolved before children. |
| `model` | Optional provider, adapter, model, and inference-mode preferences. |
| `prompts` | A normal prompt layer with `mode`, `order`, `disabled`, and `inline`. |
| `permission` | A normal permission overlay: paths, network, tools, and approval-model selection. |
| `max_turns` | Model-turn limit for a stable run. `0` means unlimited; omission inherits the parent or runtime default. |

Names follow prompt-fragment identifiers: 1–64 ASCII characters from `a-z`,
`0-9`, `.`, `_`, and `-`, with no leading dot. There can be at most 256 profiles
and 64 profiles in one inheritance chain. Unknown fields, missing parents,
cycles, invalid permissions, and unresolved prompt references are configuration
errors, including in profiles that are not currently selected.

## Select a profile from the CLI

One-shot commands send the selection to the long-lived server when creating
their session:

```bash
agena exec --profile developer "Implement the requested change"
agena review --profile reviewer --base main
AGENA_PROFILE=reviewer agena exec "Review the current changes"
```

An explicit `--profile` takes precedence over `AGENA_PROFILE`. If neither is
provided, a new ordinary root session uses `default_profile`. Pass an empty value to
start without a profile even when a default is configured:

```bash
agena exec --profile="" "Explain the current architecture"
```

You can create a session without starting a model run, then change or clear
its profile later:

```bash
agena sessions create --workspace . --title "Code review" --profile reviewer
agena sessions update 42 --profile developer
agena sessions update 42 --profile=""
```

`sessions create` also accepts `AGENA_PROFILE`. `sessions update` requires an
explicit `--profile` argument; a shell environment variable does not change an
existing conversation. Both commands print the resulting execution state as
JSON by default, including `execution.profile` when a profile is selected.

`sessions create --parent 42` creates a child session. Without an explicit
profile, it inherits the parent's selected profile, including a parent that
has no profile. A new ordinary root session uses the configured default instead.
Managed channel sessions and internal Bot roots keep their own initialization;
they can select a profile explicitly after creation.

Inspect the server's resolved configuration, including the profile catalog,
with:

```bash
agena config resolve --format json
```

## Manage and select profiles in the Web client

Open **Settings → AI & sessions → Agent profiles** (Chinese:
**设置 → AI 与会话 → 智能体配置**).
The global and workspace scopes manage their own catalogs. Create a named
profile, choose its parent when needed, and edit its description, system
instructions, model preferences, permissions, and turn limit. The instruction
editor can reference saved prompts; advanced JSON editors preserve the full
prompt-layer, model, and permission contracts.

Save the profile before choosing it as the scope's default. A workspace entry
with an existing global name is a complete override. Removing that workspace
entry reveals the global definition again. Move a default or inheritance
reference before deleting the profile it names.

Choose **Current session** to apply or clear a profile for the conversation
currently open in the client. The conversation composer also links to this
selector. Global conversations list the global catalog; project conversations
list the merged global and workspace catalog. The page remains usable when a
saved selection no longer exists, so another profile can be selected.

Saves validate the configuration and reload the runtime, with revision checks
that preserve drafts when another editor has changed the file. Profile changes
refresh the cached session execution state, including inherited model choices;
explicit session overrides remain in effect.

## Select a profile in the TUI

Open **Settings → Runtime & Session → Agent profile** for the current or
selected session. The picker shows titles, identifiers, and descriptions from
the server's current profile catalog. Global Bot sessions use the global
catalog; workspace sessions also see project definitions. Choosing a profile sends a session
update to the server and refreshes the effective execution state. Choose
**No profile** to clear the selection. The execution status includes the
selected profile name.

The picker remains available when a previously selected profile has been
removed from configuration, so you can choose an existing profile or clear the
selection. A profile change requires an idle session: finish or stop active
work and resolve queued work or pending interactions before switching.

## Configuration and inheritance

Global and project catalogs merge by name. A global definition must resolve
its parent profiles, model routes, and prompt references using global
configuration; workspace definitions may reference both catalogs. A workspace
prompt with the same ID can still override a global fragment during workspace
execution.

A project entry with the same profile name **replaces the complete global
entry**; unspecified fields do not survive from
the replaced definition. This prevents an apparently small project profile
from accidentally retaining broader permissions. Use a differently named
profile with `extends` when you want explicit reuse:

```json
{
  "default_profile": "project-reviewer",
  "profiles": {
    "project-reviewer": {
      "extends": "reviewer",
      "description": "Review this repository using its local conventions.",
      "prompts": {
        "order": ["project-review-notes"],
        "inline": [
          {
            "id": "project-review-notes",
            "body": "Check the repository's migration and compatibility conventions."
          }
        ]
      }
    }
  }
}
```

Within an inheritance chain, child model preferences and permission fields
overlay their parent's values. An explicitly specified child `max_turns`
replaces the parent's limit. Prompt layers remain separate and execute in
parent-to-child order.

The execution defaults are composed in this order:

1. Instance and workspace runtime configuration.
2. The selected profile, including its parents.
3. Explicit session settings and explicit run options where supported.

Changing a profile preserves the session's explicit settings. For example, a
model selected directly for the session continues to override a profile's
preferred model. Existing authorization principals and permission ceilings
remain in force.

### Model preferences

`model` is an object using Agena's normal model-selection fields:

```json
{
  "profiles": {
    "careful-review": {
      "extends": "reviewer",
      "model": {
        "thinking_mode": "high"
      }
    }
  }
}
```

This keeps the inherited model and requests its `high` thinking mode. Use a
mode supported by the selected route. To select a route, add `provider`,
`adapter`, and `model`; `speed_mode`, `verbosity`, and `parallel_tool_calls`
are also supported. Changing a provider, adapter, or model follows normal
selection cascading, clearing stale subordinate route and variant choices.
The provider must already be configured for the instance.

### Prompt ordering

The configured system-prompt fragments are applied in this order:

1. Global prompt layer.
2. Workspace prompt layer.
3. Matching model prompt overlays.
4. Profile prompt layers, from the oldest parent to the selected child.
5. Explicit session prompt layer.
6. Explicit request prompt layer, when supplied for the run.

`append` keeps previously resolved fragments. `replace` removes earlier
configured fragments before adding the current layer. `disabled` hides
inherited fragment IDs. Agena's built-in identity, tool instructions, and
project guidance remain part of the system prompt.

Use `order` to reference entries from the existing global/workspace prompt
library, or use `inline` for profile-owned fragments. Profile `prompts` is a
`PromptLayer`, not a separate library or model-overlay collection. See
[Layered prompt fragments](prompt-layers.md) for conditions, variables,
reference rules, and previews.

### Permissions

Profile permissions use the same schema and enforcement path as ordinary
session permissions. Filesystem values are `readonly`, `readwrite`, or `deny`.
Tool approval values are `allow`, `ask`, `auto`, or `deny`; tool rules use their
canonical names, for example `agena.shell.exec`. Network policy uses the
existing network-permission schema.

The review example makes the workspace read-only for a session that has no
explicit permission override. Session overrides are applied after the profile;
review or clear those overrides when adopting a restrictive profile in an
existing session. A profile does not replace the instance's access-control
directory or a session's permission ceiling. See [Sandbox configuration](sandbox.md)
for filesystem and network policy details.

Delegated agents retain the parent's effective permissions at delegation time,
together with every inherited permission ceiling. Those limits remain in force
when the child changes profiles or a profile definition is reloaded.

## Session API

Create a session with `POST /api/v1/sessions`:

```json
{
  "workspace_id": 1,
  "title": "Code review",
  "profile": "reviewer"
}
```

Change a session with `PUT /api/v1/sessions/42`:

```json
{
  "profile": "developer"
}
```

To clear the profile, send `{"profile": ""}`. Omitting `profile` when updating
preserves the saved selection; omitting it when creating applies the root
default or parent inheritance described above. The execution-state response
exposes the saved selection as `execution.profile` (omitted when none).
Switches that would affect active, queued, or interaction-blocked work fail
with a conflict response instead of changing its configuration midway.

## Persistence and configuration reload

Sessions save the selected profile **name**, while configuration owns its
definition. Editing a profile updates its later resolutions after settings
reload; it does not copy another stale prompt or permission object into every
session. Settings API writes can request validation and reload. Direct edits
to configuration files take effect when the server reloads configuration or
restarts.

Changing `default_profile` affects new ordinary root sessions. Existing sessions keep
their saved name, and sessions with no profile remain without a profile. Forks
and ordinary child sessions carry their inherited selection.

If a selected profile is removed, subsequent resolution reports the missing
profile. Restore the definition or explicitly select another profile or clear
the session's profile. This also applies to an imported session whose selected
profile is absent from the destination catalog.
