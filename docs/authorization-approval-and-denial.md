# Authorization, approval, and non-execution outcomes

This document defines Agena's single authorization architecture. It is a
breaking contract: callers must consume the outcome unions described here;
there is no compatibility path that turns authorization into a generic error
or accepts a caller-supplied `user_approved` boolean.

## Invariants

1. Every protected action is collected before execution and resolved by one
   policy resolver.
2. `Allow` executes without interaction.
3. `Ask` never executes immediately. For a session-scoped call it creates a
   durable `PermissionRequest`; the exact invocation resumes only after the
   matching reply. A sessionless protocol receives a normal
   `approval_required` result and performs no side effect.
4. `Deny` never opens an approval prompt and never executes. It produces a
   structured `policy_denied` tool result identifying the effective authority
   and, for stored rules, the rule row and revision.
5. A user's refusal of an existing request is `user_declined`, not policy
   denial and not tool failure.
6. A missing runtime capability is `capability_unavailable`. A missing or
   unloaded named tool is `tool_unavailable`. Neither is a permission decision
   and neither can be fixed by approving the call.
7. OS ACL errors, process errors, network failures, HTTP authentication/CSRF,
   plugin crashes, and invalid tool input are execution failures. They do not
   become Agena policy outcomes.
8. Provider, REST, RPC, Web, TUI, CLI, MCP, and plugin Host API boundaries do
   not receive raw Ask/Deny errors. They receive typed normal outcomes.
9. Execution after authorization requires an opaque `ExecutionGrant`. No API
   can request a generic permission bypass.
10. AI providers receive only the five fixed Agena Tool API gateway
    functions. Execution tools are never declared directly to the model and
    provider-native conversation tools are never injected by session
    composition; both would create a second route around the resolver.

## State machine

```text
resolve tool and hard capability
  ├─ tool missing ------------------------------> ToolUnavailable
  ├─ capability/profile cannot execute --------> CapabilityUnavailable
  └─ capability exists
       └─ collect every static + declared + dynamic action
            └─ resolve persisted/static/plugin policy
                 ├─ any Deny -------------------> PolicyDenied
                 ├─ any Ask
                 │    ├─ session: persist request and wait
                 │    │    ├─ allow ------------> issue exact grant -> execute
                 │    │    └─ decline ----------> UserDeclined
                 │    └─ sessionless -----------> ApprovalRequired (no execution)
                 └─ all Allow -------------------> issue exact grant -> execute
                                                      ├─ success -> Completed
                                                      ├─ cancel  -> Cancelled
                                                      └─ error   -> Failed
```

`PolicyDenied`, `UserDeclined`, `CapabilityUnavailable`, and
`ToolUnavailable` are terminal non-execution states. They are projected to an
AI provider as a completed tool-protocol frame containing a structured result,
so the model can continue normally without interpreting the result as a
transport or provider failure.

## Exact execution grants

An `ExecutionGrant` binds all of the following:

- session ID;
- call ID;
- digest of the canonical prepared invocation;
- digest of the prepared shell command and working directory, when present;
- the complete set of authorized `PermissionAction` values.

The executor recollects protected actions and validates every binding at the
last execution boundary. Changing the invocation, prepared command, identity,
or action set invalidates the grant. Grants are created only after resolution
or an explicit matching approval. Runtime-discovered Host API actions are kept
in a per-invocation ledger, are approved independently, and are removed when
the outer invocation guard drops.

## Outcome contracts

### Policy denial

```json
{
  "status": "policy_denied",
  "code": "permission_policy_denied",
  "retryable": false,
  "denial": {
    "action": {},
    "related_actions": [],
    "denied_actions": [],
    "reason": "...",
    "explanation": "...",
    "authority": "persisted_rule",
    "source": "permission_studio",
    "scope": "workspace",
    "operator": "user-id",
    "rule_id": 42,
    "rule_revision_ms": 1785438000000,
    "risk": "high",
    "trace": []
  }
}
```

`authority` is one of `static_policy`, `persisted_rule`, or `plugin_policy`.
Stored rules carry `rule_id` and `rule_revision_ms`; static and plugin policy
do not invent a database identity.

### User decline

```json
{
  "status": "user_declined",
  "code": "permission_request_declined",
  "retryable": false,
  "decline": {
    "request_id": "...",
    "action": {},
    "related_actions": [],
    "reason": "...",
    "persisted_scope": null
  }
}
```

### Availability

`capability_unavailable` identifies the capability, tool when applicable,
reason, hard-boundary source, and whether a configuration/platform change can
make it retryable. `tool_unavailable` identifies the requested name,
suggestions, registry/load source, and retryability. Approval is never offered
for either result.

## Durable approvals

Session-scoped external UI/API calls create an assistant operation and a
permission request in the same session projection. The response exposes the
request ID. Existing permission reply APIs are the only way to approve or
decline it. The invocation and action set are reconstructed from the durable
operation; the client cannot submit an altered invocation with an old approval
ID. External-tool replies execute the tool but do not start an unrelated model
continuation.

Dynamic plugin Host API checks use the same durable request/reply mechanism.
Multiple actions discovered by one plugin call can therefore generate multiple
ordered approvals. Completing a request part does not mark its owning tool
operation complete; only a terminal operation part closes the tool.

## Boundary ownership

- Agena policy resolver: user/static/plugin Allow, Ask, and Deny.
- Capability resolver: agent profile, execution-access profile, model tool
  profile, build, platform, and runtime availability.
- Operating system: filesystem ACL, process launch, socket, and device access.
- HTTP server: authentication, authorization of HTTP principals, and CSRF.
- Tool runtime: input validation and actual execution failures.

These authorities remain distinct in status, payload, logs, and UI styling.
In particular, an OS `EACCES` is never presented as a user-configured Agena
`PolicyDenied`, and an HTTP 401/403 never creates a tool approval request.

## Persistence and events

SQLite stores distinct execution status values for policy denial, user
decline, capability unavailability, and tool unavailability. Tool completion
and its specialized policy-declined/user-declined event are appended as one
history batch. Permission-rule writes and session projection changes use the
existing transactional session commit path.

## Removed contracts

The refactor deliberately removes:

- generic permission-bypass executor methods;
- `PermissionEnforcementMode::Bypassed`;
- caller-provided `user_approved` flags;
- raw `PermissionAsk` and `PermissionDenied` tool errors;
- the unused second `PermissionRuntime` and `PermissionRuleStore` engine;
- direct sessionless runtime-tool execution that skipped persisted policy;
- hiding a present tool merely because user policy says Deny;
- legacy direct-tool presentation config (`agena_tools.direct`), direct
  `execution_tool` provider bindings, and replay-only
  `provider_function_name` identities;
- compatibility composition fields for conversation-level provider-native
  tools.

Callers must migrate to the typed outcome unions and the durable permission
reply flow.
