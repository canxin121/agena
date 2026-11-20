# Agent profiles

Profiles define the role of a running agent: its responsibilities, system-prompt
fragments, model preferences, permissions, and turn limit. Agena supplies working
roles in an empty configuration, exposes suitable roles to the model, and applies
the selected role when creating a child agent.

## Built-in roles

| Profile | Mode | Responsibility |
| --- | --- | --- |
| `main` | `primary` | Own the user goal, coordinate specialists, integrate work, verify, and deliver. |
| `explore` | `subagent` | Read-only discovery of code, behavior, and sources; return evidence and recommendations. |
| `worker` | `subagent` | Complete a bounded implementation, fix, or other substantive assignment. |
| `reviewer` | `all` | Independently review correctness, regressions, authorization boundaries, and test gaps. |

`explore` and `reviewer` have read-only filesystem defaults and a 40-turn limit.
`main` and `worker` use the surrounding permission and turn-limit defaults. Roles
use configured models; a delegated role without its own model can use the parent's
actual model as a fallback.

The common system layer contains identity, evidence and authorization requirements,
tool protocols, conversation routing, and output-format rules. Task methods and
role responsibilities come from the selected profile's prompt layer. An explorer
therefore receives exploration instructions, rather than a copy of the parent's
main-role workflow.

## Active role selection

The default configuration is equivalent to:

```json
{
  "default_profile": "main",
  "default_subagent_profile": "worker"
}
```

`default_profile` chooses the saved role for a new ordinary root session.
`default_subagent_profile` chooses the role for a new delegated agent when the
caller omits `profile`. Set either to a compatible custom role. An empty string
resets an inherited configured default to `main` or `worker`, respectively;
`null` or omission inherits the broader configuration's value.

A stored selection remains attached to its session through reconnects, restarts,
forks, and rewinds. Updating a configured creation default does not replace a
saved selection.

### Existing sessions and restoring the default role

An unselected session has an effective role: `main` for ordinary conversations,
`worker` for subagents. This also applies to existing sessions whose stored
`profile` is absent. Sending `profile: ""` clears the saved selection and restores
this default role. It does not remove the agent's task instructions.

Execution state reports the effective name in `execution.profile` and sets
`execution.profile_is_default` when the name comes from this unselected-session
fallback. The clients show the effective role and distinguish it from a saved
selection. This updates the earlier clear-profile behavior, which left a session
without any profile layer.

Switching a profile requires an idle session. Active, queued, or
interaction-blocked work returns a conflict. If a saved name no longer exists,
restore its definition or select another role, including restoring the default.

## Model-driven delegation

When the agent can use `agena.agent.spawn`, its system context contains the
current delegation catalog: names, descriptions, and the configured default.
The catalog is resolved from the same scope and execution snapshot used for the
run, and includes custom roles. Only roles that support subagent use and are not
hidden appear in discovery.

The main role uses this catalog to assign independent exploration, implementation,
and review work. A useful delegation specifies the role, the task, relevant
context, ownership boundaries, and expected output:

```json
{
  "name": "trace_session_flow",
  "profile": "explore",
  "prompt": "Trace session creation through storage and execution. Return the relevant files, invariants, and likely extension points. Keep this investigation read-only."
}
```

```json
{
  "name": "implement_session_change",
  "profile": "worker",
  "prompt": "Implement the agreed session change in the assigned modules and verify the affected behavior. Preserve concurrent work. Report changed files and test results."
}
```

Use `reviewer` for an independent review of the result. `agent.spawn` validates the
name and role mode, and its handle returns the actual selected profile. A
primary-only role cannot be launched as a subagent. Omitting `profile` uses the
configured subagent default; an explicit empty value uses `worker`.

Continue an existing assignment with `agent.followup`. Followup preserves the
child's own profile, explicit settings, and permission boundaries. It does not
reset the child to the parent's role.

### Delegated models and permissions

A new child resolves model selection in this order:

1. Runtime model defaults.
2. The parent's actual model and modes, saved as low-priority agent defaults.
3. The selected child profile's model preferences.
4. The child's explicit model settings and explicit spawn or run parameters.

The inherited fallback is separate from explicit selections. A later profile
model update can therefore take effect without being pinned by the parent's
previous choice. A child profile can set only inference modes while retaining a
compatible inherited route.

To select another model route, set both `provider` and `model` in the profile.
Leaving `adapter` absent then lets that provider resolve the adapter for the
chosen model. A model-only override retains the inherited provider and adapter;
changing provider, adapter, or model clears inherited inference modes before
applying the profile's modes and the chosen model's defaults.

A child starts with its own role's permission defaults. Parent permission
overrides are not copied over those defaults. Instead, the parent's effective
permissions and every inherited ceiling remain independent limits; the selected
role's declared permissions are also retained as a delegation boundary. These
limits are enforced by the tool executor, survive reloads and restarts, and
cannot be removed by changing the child's profile. Reusing a child preserves
previous boundaries and adds any newly required restriction.

## Define and customize roles

Put shared configuration in the instance file, normally `~/agena/agena.json`
(or `<data-dir>/agena.json` with `--data-dir`). Project configuration belongs in
`<workspace>/.agena/agena.json`.

A profile supports:

| Field | Meaning |
| --- | --- |
| `title`, `description` | Display name and purpose. Descriptions also help the model choose a child role. |
| `mode` | `primary`, `subagent`, or `all`. A new custom profile defaults to `all`; inherited roles keep their parent's mode unless overridden. |
| `hidden` | Hide from model-facing delegation discovery. Explicit selection and a configured default remain supported. |
| `extends` | One parent profile; instructions and defaults resolve parent first. |
| `model` | Provider, adapter, model, thinking/speed modes, verbosity, and parallel-tool preference. |
| `prompts` | A prompt layer with `mode`, `order`, `disabled`, and `inline`. |
| `permission` | The standard path, network, tool, and approval-model permission overlay. |
| `max_turns` | Model-turn limit; `0` means unlimited, omission inherits. |

### Customizing a built-in

Changing a built-in's model or limit keeps its other factory settings. For
example, this changes the exploration budget while retaining its role prompt,
mode, description, and read-only permissions:

```json
{
  "profiles": {
    "explore": { "max_turns": 24 }
  }
}
```

Built-in fields provide defaults under the final user definition. Permission
fields overlay their built-in values. Prompt `append` keeps the built-in
fragments and adds the supplied fragments; `replace` explicitly replaces the
configured role layer. `disabled` can suppress a built-in fragment such as
`agena-profile-main`. Titles and descriptions inherit when empty; optional
fields inherit when omitted. `builtin_profiles` in resolved settings exposes the
factory definitions separately so editors can restore an override.

### Creating a specialized role

```json
{
  "profiles": {
    "repo-explorer": {
      "extends": "explore",
      "title": "Repository explorer",
      "description": "Trace architecture, storage boundaries, and extension points before implementation.",
      "prompts": {
        "order": ["repo-exploration"],
        "inline": [
          {
            "id": "repo-exploration",
            "body": "Include the relevant tests and public interfaces when explaining a code path."
          }
        ]
      }
    }
  }
}
```

The inherited mode makes this a subagent role. It immediately appears in the
model's delegation catalog after configuration reload. To make a custom role
available both directly and through delegation, set `mode: "all"`.

Global and workspace **user entries** still compose by name: a workspace entry
replaces the entire global user entry of the same name. Factory defaults are
applied after these file layers have composed. Use `extends` with a different
name when you want explicit reuse of another custom definition.

Mode checks govern new explicit selections and delegation. Saved roles remain
usable after a mode change and through inherited forks or rewinds; they can also
be cleared to restore the relation's default role.

The fallback names `main` and `worker` must remain compatible with primary and
subagent use, respectively. Creation defaults must exist and support their scope.
Names use 1–64 ASCII characters from `a-z`, `0-9`, `.`, `_`, and `-`, with no
leading dot. The final catalog has at most 256 entries and an inheritance chain
at most 64 profiles. Cycles, invalid modes, unknown parents, invalid permissions,
and unresolved prompt references are rejected even for unselected roles.

## Prompt and explicit-setting precedence

Configured prompt fragments apply in this order:

1. Global prompt layer.
2. Workspace prompt layer.
3. Matching model overlays.
4. Profile layers, parent to child.
5. Explicit session prompt layer.
6. Explicit request prompt layer.

`replace` removes earlier configured fragments; the common execution contract,
project guidance, and tool protocols remain. See [Layered prompt fragments](prompt-layers.md)
for conditions, variables, and previews.

For an ordinary session, runtime defaults are followed by profile defaults and
then explicit session settings. Explicit model and permission overrides remain
in effect when changing profiles. Clearing an explicit model selection restores
the profile's preferred model. Inherited defaults are never saved as if the user
had manually selected them.

## Clients and API

The Web entry is **Settings → AI & sessions → Agent profiles** (Chinese:
**设置 → AI 与会话 → 智能体配置**). It shows factory roles and their overrides,
separate primary/subagent creation defaults, mode and discovery visibility,
instructions, models, permissions, and limits. Editing a factory role writes a
user override; deleting that override restores the underlying definition.
Settings saves validate and reload with revision checks that preserve drafts on
conflict. The current-session selector shows the effective role and supports
restoring the default role.

The composer's Profile chip opens a searchable picker in place using the shared
`OptionMenu`. Choose a compatible role or **Restore default role**; selecting an
option saves it and refreshes the complete execution state so the displayed
model and permissions update together. **Default profile settings** at the
bottom of the picker opens the current session's workspace or global defaults.

To set creation defaults, choose **Global layer** or **Workspace layer**, then
use the **Default profiles** section at the top of the page. It has separate
selectors for new ordinary sessions and subagents, each showing its currently
effective value and source. **Inherit** shows the broader layer's default;
**Built-in default** chooses `main` or `worker`. Save each default separately.
The **Current session** view includes **Global default profiles** and
**Workspace default profiles** links to these settings. Searching settings for
“default profile” also finds the page.

The TUI entry is **Settings → Runtime & Session → Agent profile**. It displays
names, descriptions, scope and source, and filters roles for the selected session's
relation. Global sessions use the factory plus global catalog; project sessions
also see workspace definitions. Hidden roles remain available for manual use.

```bash
agena exec --profile main "Implement the requested change"
agena review --base main
AGENA_PROFILE=reviewer agena exec "Review the current changes"
agena sessions create --workspace . --title "Code review" --profile reviewer
agena sessions update 42 --profile main
agena sessions update 42 --profile=""
agena config resolve --format json
```

`agena review` defaults to `reviewer`. `AGENA_PROFILE` overrides this command
default, and an explicit `--profile` overrides the environment variable. Passing
`--profile=""` uses the automatic `main` role, including for `review`.

`AGENA_PROFILE` also applies to `exec` and `sessions create`. `sessions update`
requires an explicit flag; its empty value restores `main` for ordinary sessions
or `worker` for subagents.

Create sessions with `POST /api/v1/sessions` and update them with
`PUT /api/v1/sessions/{id}` using `{"profile":"reviewer"}`. On update, omission
preserves the saved selection and `{"profile":""}` restores the relation's
default role. On creation, omission uses the configured creation default or the
ordinary parent-session inheritance rules.

Profile definitions stay in configuration; sessions save their selected name.
Reloading settings affects later resolutions of the selected role, while a run
uses its execution snapshot. Direct file edits take effect after configuration
reload or restart. The separate `plugins.profiles` setting continues to describe
plugin deployment, and is not this agent-role catalog.
