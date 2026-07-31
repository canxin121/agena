# Failure semantics

Agena treats failure handling as an audience-routing contract, not as string
formatting. A failure occurrence has one `FailureId` and one machine meaning,
but its user, model, and diagnostic representations are different types.

## Outcomes are not failures

Accepted commands return an execution receipt. Completion, cancellation,
permission waiting, user-input waiting, and already-terminal cancellation are
execution outcomes or state transitions. They do not create a public failure.
In particular, cancellation is represented by `ExecutionOutcome::Cancelled`;
there is deliberately no `FailureCategory::Cancelled`.

## Audience routing

`Failure` is the durable internal semantic record. It may contain a closed
`ModelFeedback` kind when a tool failure has a safe model recovery path.

`UserProblem` is the only failure projection allowed in REST, WebSocket, SSE,
IPC, JSON-RPC, TUI/Web/VS Code resources, runtime status, and other
user-facing protocols. It preserves the originating `FailureId`, category,
retry/recovery policy, impact, and user presentation, while omitting model
feedback and all diagnostics.

`ModelFeedback` is opt-in and closed. It contains no producer-supplied prose;
the model message is rendered from a known semantic kind. Provider bodies,
plugin diagnostics, paths, SQL, backtraces, and cancellation propagation must
never be converted into model feedback.

Diagnostics remain process-local. The producing layer logs the original
source with the same `FailureId`. Adapters preserve that ID and must not wrap a
structured failure in a newly generated internal failure. Diagnostic values
are never persisted in public status, transcript, scheduler, background-task,
subtask, or protocol records.

## Presentation rules

Expected failures use short, actionable prose. Internal and data-corruption
failures use a generic message plus `Reference: <FailureId>`. Machine codes are
for branching and telemetry, not primary UI copy. Field validation belongs at
the field or form; tool failures belong on the tool call; background failures
belong in task status; cancellation uses a neutral notice rather than an error
toast. Replayed events with the same `FailureId` must not create duplicate
notices.

Public expected-failure constructors accept only `&'static str` presentation
copy. A parser error, provider message, command stderr, path, identifier, or
other dynamic value must use a `*_with_diagnostic` constructor: its static
fallback enters `UserProblem`, while the dynamic value is logged against the
same `FailureId`. This is a compile-time boundary, not a caller convention.
The TUI likewise has no implicit `String -> UiFailure` conversion, and
transcript activities carry `UserProblem` rather than `Option<String>` error
text.

Provider Studio's catch-all authentication and settings failures are created
as `UserProblem` values in the backend. Their OAuth, HTTP, serialization, and
configuration details are logged at creation and cannot be recovered by the
TUI renderer. Localized, product-owned validation copy remains a deliberate
UI message; external or library errors use a fixed fallback plus diagnostic.

## Persistence and concurrency

Persistence stores semantic failures only. Diagnostic strings are forbidden
in public failure columns. Projection writes and their watermark commit in the
same transaction; watermarks advance monotonically; terminal event replay is
idempotent; cancellation cannot be overwritten by a late provider or tool
failure. Subtask cancellation stores a cancelled terminal state without a
failure payload.

## Untrusted producers

Plugin and MCP data are untrusted. Hosts classify their error kind, retain the
original detail only in correlated diagnostics, and generate canonical user
and model projections. A plugin cannot choose UI copy or inject a model
message by serializing a custom `Failure`.

## Architecture gate

Run the failure-semantics invariant check with:

```bash
python3 scripts/refactors/check-refactor-invariants.py \
  --manifest scripts/refactors/failure-semantics-invariants.json \
  --root .
```

The gate prevents reintroduction of cancelled failures, user protocols that
serialize internal `Failure`, free-form model feedback, public string status
errors, the old plugin error constructor, legacy session-creation and stream
error strings, implicit TUI string promotion or failure demotion, free-form
transcript activity errors, dynamic public failure constructors, and Web
catches that render arbitrary exception messages.
