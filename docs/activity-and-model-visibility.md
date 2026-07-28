# Activity and model visibility

Agena treats conversation content and user-visible application activity as
different data types.

## Invariants

- `PartContent::Activity` is durable transcript/application state. It never
  enters a provider request, prompt-window digest, token estimate, conversation
  parent calculation, or doom-loop detection.
- `PartContent::Operation` is model tool-protocol state. Tool calls and their
  model-safe results may enter provider history.
- `MessageSource` and `Role` describe provenance and protocol role. Neither is
  a model-visibility control.
- Provider projection returns `None` for an activity-only message. An activity
  therefore cannot survive as an empty provider message.
- Control-plane failures are stored in `ActivityPart::error`. They are visible
  to the user and absent from model input.
- Tool failures are different: a model-safe tool-result summary may be sent to
  the model so it can recover. UI details and provider-private state remain
  separately typed.

## Execution lifecycle

Every registered session execution persists one activity identity at
`ExecutionStarted` and updates the same identity at `ExecutionFinished`:

```text
InProgress -> Completed | Failed | Cancelled
```

Normal submitted-message activity remains latent while the optimistic user
message is visible. It becomes visible if the execution fails or is cancelled.
Continue, compact, permission-reply, and user-input-reply activity is visible
while running.

Manual compaction enriches its existing execution activity with typed
`PromptCompactionActivity` details. Automatic/reactive compaction creates a
separate completed activity because the surrounding model execution continues.

## Compaction data boundary

The UI activity (status, token reduction, strategy, generation, and errors) is
never model-visible. Historical messages sent to a summarizer and the resulting
checkpoint are conversation-context data and are intentionally model-visible
according to the selected compaction strategy.

## Adding new activity

New user-only lifecycle data must use `PartContent::Activity`. Do not encode an
activity as system text or operation metadata. Add projection, API, renderer,
and provider-exclusion tests together.

## Development database boundary

This change is intentionally incompatible with schema-version 1 SQLite
databases. New databases use `PRAGMA user_version = 2`. Agena rejects an older
or newer version with an explicit error and never migrates or deletes it
implicitly; create a fresh development database when adopting this format.
