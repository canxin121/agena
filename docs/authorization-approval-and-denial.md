# Model tool permission

This is Agena's only model-tool permission system. It applies exclusively to
real tool invocations returned by an AI provider. It is intentionally not a
general authorization framework for application commands, plugin callbacks,
REST handlers, MCP clients, or the UI.

## One invocation, one authority

The provider-facing Tool API has four discovery functions (`tools_list`,
`tools_search`, `tools_help`, and `tools_tags`) and the `tools_call` gateway.
When a provider returns `tools_call`, Agena preserves that outer function and
call ID for provider replay, but immediately decodes its `{ tool, input }`
envelope into the real target invocation. From that point onward the target
name and input are the authoritative operation:

```text
provider tools_call { tool: "fs.write", input: {...} }
  └─ decode before permission evaluation
       └─ fs.write {...}
            └─ central model-tool policy and execution
```

There is no executable `agena.tools.call` plugin handler, nested Host API
invocation, execution grant, approval ledger, thread-local executor context,
or plugin permission-decision hook in this path. These would introduce a
second authority or lose the identity of the actual operation.

## State machine

```text
real provider tool invocation
  ├─ prepare / validate target
  ├─ collect static, persisted, and plugin-declared actions
  └─ central policy result
       ├─ Allow ────────────────────────────────> execute the target
       ├─ Ask ──> persist Permission Activity ──> wait for user reply
       │                                             ├─ allow ─> record approved actions
       │                                             └─ decline -> user-declined activity
       └─ Deny ─────────────────────────────────> policy-denied activity
```

`Ask` never executes speculatively and never becomes a tool result sent back
to the model. The response is paused while the user sees a durable Permission
Activity. A matching approval resumes the original prepared target; it does
not ask the model to retry or submit a changed invocation. `Deny` never opens
an approval prompt and never executes.

For one provider-emitted batch, Agena prepares every member and persists every
independently actionable Permission Activity before it blocks. Permission
replies then form a barrier:

1. each reply is persisted against its exact assistant message and Operation;
2. no approved member starts while a sibling Interaction remains pending;
3. the final reply starts one continuation execution for the existing
   canonical assistant reply;
4. concurrency-safe approved tools fan out together, while sequential tools
   retain transcript order;
5. the model continues only after the entire pending tool batch settles.

The completed Permission Activity is the durable one-shot authorization
token, but only for the actions recorded in that request. Current checks with
new actions still pass through central policy evaluation. Matching also
includes the owning assistant message, so a provider call ID reused in a later
turn can never inherit an old approval.

The activity and provider tool-result projections retain the actual target,
input, structured output, and a safe actionable error. Generic messages such
as “the plugin reported an error” or “the tool provider failed unexpectedly”
are not valid terminal presentations.

## What is not model-tool permission

Application-initiated UI actions, plugin commands, Host callbacks, REST/MCP
operations, and other non-model work execute through their own normal runtime
contracts. They do not create model permission requests and cannot produce an
approval result for an AI model. OS ACLs, HTTP authentication, input
validation, runtime capability checks, and plugin failures are likewise not
policy decisions.

Plugins can declare the filesystem paths or network targets a model tool will
use. Those declarations are inputs to the central evaluator, not a separate
plugin permission engine. Public-network validation performed inside a tool is
an SSRF/input-safety constraint, not user approval.

## Terminal outcomes

- `policy_denied`: central policy rejected the real target; no side effect.
- `user_declined`: the user declined the durable Permission Activity; no side
  effect.
- `capability_unavailable` / `tool_unavailable`: runtime availability issue,
  not a permission decision.
- execution failure: target validation, OS, process, network, provider, or
  plugin failure. Its safe diagnostic is presented as the tool failure.

All terminal execution states are Activity-backed so the transcript preserves
their ordering with surrounding message text. Provider replay remains faithful
to the provider function envelope, while execution and UI remain faithful to
the real target.
