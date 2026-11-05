# Plugin hooks

Agena hooks belong to the runtime phase that produces their input. The
`Plugin` trait, `#[hook(...)]` macro, manifest subscriptions, and host
dispatch all refer to the same typed event contracts. A plugin declares only
the events it handles; the host dispatches only subscribed events.

## Describe your hooks

Every `#[hook(...)]` method **must** declare a non-empty `summary` and
`help`. Missing or empty fields fail at compile time, including lifecycle
hooks like `init` and `shutdown`. Each handler introduces itself in the
plugin inspector independently of other methods on the same event:

```rust
#[hook(
    tool.before,
    tool = "bash",
    priority = 100,
    summary = "Check shell commands",
    help = "Checks a shell invocation before the tool is allowed to run.",
    translations(
        locale(
            "zh-CN",
            summary = "检查 Shell 命令",
            help = "在执行前检查 Shell 工具调用是否符合策略。"
        ),
        locale(
            "fr-FR",
            summary = "Contrôler les commandes shell",
            help = "Vérifie la commande shell avant son exécution."
        )
    )
)]
async fn check_shell(&self, input: ToolBeforeInput) -> Result<Option<ToolBeforePatch>> {
    // ...
}
```

`summary` is the short UI title; `help` explains this specific handler's
behavior. Neither value replaces the event identifier or changes permissions.
`translations(locale(...))` uses BCP-47 locale identifiers. Every declared
locale must also supply **both** non-empty `summary` and `help`, or macro
expansion fails. Translations are optional as a whole; the default `summary`
and `help` should be in English. Locale resolution tries the exact tag, its
language, `en-US`, `en`, and finally the authored defaults. Missing locales
never hide a fully documented handler.

**Built-in plugins have a stricter localization contract:** every bundled
Hook must provide both `summary` and `help` for every language supported by
the Web Settings or TUI interface. English is supplied by the required
default fields. The test
`capability_manifest::hook_description_tests::every_bundled_hook_translates_summary_and_help_for_all_product_locales`
discovers both locale catalogs, checks every bundled and runtime-provided
Hook manifest, and compares the registered Handler count with the Hook
attributes in the source tree. A new built-in Hook or a newly supported
interface language therefore requires complete translations rather than
silently falling back to English.

Macro expansion preserves a distinct `HookHandlerDefinition` in
`PluginManifest.hook_handlers` for each annotated method, including its
stable method ID, event, priority, filters, sync/async mode and translations.
Plugin authors using another transport may supply the same manifest schema
explicitly. A stopped plugin can still present its declared documentation;
the Host contributes current registration, provenance and trust state
separately. Multiple methods sharing one `tool.before` subscription are
shown as separate handler entries. Plugins own their descriptions; an
undocumented event subscription alone is not a registered Handler.

For stdio, HTTP, or Wasm plugins, the corresponding manifest fragment is:

```json
{
  "hooks": ["tool.before"],
  "hook_handlers": [
    {
      "id": "check_shell",
      "event": "tool.before",
      "summary": "Check shell commands",
      "help": "Checks shell commands before execution.",
      "priority": 100,
      "filters": { "tools": ["bash"] },
      "translations": {
        "zh-CN": {
          "summary": "检查 Shell 命令",
          "help": "在执行前检查 Shell 命令。"
        }
      }
    }
  ]
}
```

The manifest's `hooks` list remains the executable subscription declaration.
Handler descriptions are presentation metadata and cannot grant capabilities
or enable a hook that was not subscribed. The Host rejects manually created
handler records with blank text, invalid event subscriptions, duplicate
event/handler IDs, or incomplete locale documentation. Bare subscription
flags without a self-documented handler are not listed as registered hooks.

## Inspecting hooks

The Web plugin workbench's **Hooks** tab groups handlers by their canonical
event type. Each row gives priority to the plugin-authored localized summary
and help; its method ID, execution mode, non-default priority and matchers are
secondary. Registration source and trust details are disclosed on demand.

The TUI plugin workbench also has a **Hooks** tab (reachable through tab
navigation or the `hooks` detail route). It shows the same per-handler
descriptions grouped by event, including Active, Inactive or Declared state,
with a compact technical line. The TUI uses the current interface locale and
the same authored fallback strings as the Web interface. Neither surface
conflates two handlers subscribed to the same event or shows a bare bitset
subscription as an executable handler.

A hook runs in the authority and lifetime of its owning plugin. Every
registered lifecycle handler is owned by a `PluginEffectScope`, so disposal
and reload remove the old generation's registrations. Guard hooks cannot grant
permissions, and observer results cannot change completed operations.

## Lifecycle

| `#[hook(...)]` | Trigger | Effect |
| --- | --- | --- |
| `init` | Plugin initialization | Initialize resources |
| `shutdown` | Plugin shutdown | Release resources |
| `session.start` | Session initialized, restored, or cleared | Initial context |
| `session.end` | Session ends | Observe |
| `run.pre` | Execution claimed, before its first effect | Observe |
| `run.post` | Execution settled (success, failure, cancel) | Observe |
| `prompt.submit` | User message submission | Modify, add context, deny |
| `agent.stop` | Agent intends to stop | Request continuation |
| `agent.cancel` | Agent cancelled | Observe |
| `config.resolved` | Plugin configuration resolved | Transform |

A **run** is a single claimed session execution, not a provider request or a
model round. `run.pre` and `run.post` each fire once for that execution,
including manual compaction executions. Their payloads contain a stable
execution ID, model, `RunKind` (`conversation` or `compaction`) and, on
completion, `RunStatus` (`succeeded`, `failed`, `cancelled`).

`agent.stop` has an explicit continuation budget and reports
`stop_hook_active` on subsequent stop-triggered attempts. This prevents
accidental recursive vetoes without changing the independent plan-autonomous
run budget.

## Model and context

| `#[hook(...)]` | Trigger | Effect |
| --- | --- | --- |
| `chat.system` | System prompt composed | Transform |
| `chat.message` | Individual provider-bound message projected | Transform / drop |
| `chat.messages` | Entire provider-bound conversation projected | Transform |
| `chat.params` | Model completion request prepared | Transform supported parameters |
| `chat.headers` | HTTP request headers resolved | Transform non-credential headers |
| `model.request.before` | Before provider network I/O | Guard |
| `model.request.after` | Model round settles (including failure/cancel) | Parallel observe |
| `model.request.error` | A failed model round | Suggest bounded retry |
| `compaction.before` | Context compaction starts | Guard |
| `compaction.after` | Compaction attempt settles | Parallel observe |

Model-input transformations use a **request-local projection**. They never
mutate committed session history. Tool calls, tool results, system notices,
and provider-native continuation state retain their original identities.
Alterations are checked and bounded before a provider request is admitted
and before token-budget evaluation.

`chat.headers` is **request scoped**, not a mutable global singleton.
Plugin callbacks cannot read or overwrite Provider-owned credentials,
including authorization headers and API keys. Both HTTP requests and lazy
stream polling use the owning session's header policy.

Only a failed round that has not produced assistant content or local tool
effects can be retried via `model.request.error`. Agena caps this recovery
at two retries per uninterrupted failure sequence. The normal Provider retry
and cancellation policies remain authoritative.

## Tools, agents, and permissions

| `#[hook(...)]` | Trigger | Effect |
| --- | --- | --- |
| `tool.definition` | Tool schema/description projected | Transform |
| `tool.before` | Before tool execution | Transform / deny |
| `tool.after` | After tool execution | Transform result |
| `tool.failure` | Tool invocation failed | Observe |
| `shell.before` | Before shell command | Transform / abort |
| `shell.after` | Shell command completed | Transform result |
| `shell.env` | Shell execution environment prepared | Transform |
| `permission.request` | Interactive tool approval needed | Guard: additional denial only |
| `subagent.start` | Delegated child launch is being prepared | Guard |
| `subagent.stop` | Child execution settles | Parallel observe |

The **Permission Engine** remains authoritative. A hook cannot turn
`deny` into `allow`; it can only deny a request before interactive
approval. `tool.before` returns modified input to the tool executor,
which validates the schema and applies actual path/network authorization
before executing the tool.

A rejected child launch is settled as a failed child attempt; it is never
silently left running. Terminating child executions publish terminal
`subagent.stop` observations after durable settlement.

## Integrations

`provider.list` transforms the visible provider catalog, and
`notification` observes host notifications. Plugins can subscribe directly
to filtered host events through `HostClient::subscribe_events`; that event
bus is separate from lifecycle interception.

`tool.invoke` and `tool.invoke.stream` are tool-provider capabilities
recorded in plugin manifests. They are not `#[hook(...)]` interception events.

## Execution and diagnostics

Lifecycle guards use `PluginGuardPipeline` and deny on handler failure.
Observers use `PluginObservePipeline` and isolate errors from the completed
operation. Model error recovery uses `PluginBailPipeline`; its decision is
accepted only at the safe retry boundary. The host's architecture catalog
exposes registered handlers and their effect-scope owners. Effective
decisions, failures and timeouts are recorded as scoped Hook activity.

The typed contracts live in `agena-plugin-sdk/src/hooks`, the effect-owned
pipelines in `agena-plugin-host/src/host/lifecycle_hooks.rs`, and the
triggering boundaries in the Session, Tool and Provider runtimes.
