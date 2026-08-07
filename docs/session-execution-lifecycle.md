# Session execution lifecycle

This document defines the ownership and consistency rules for session execution. These are architectural invariants, not UI conventions.

## Three independent state machines

The system deliberately does not expose one overloaded `status` field.

| State machine | Owner | Question it answers | Durable source |
| --- | --- | --- | --- |
| `WorkflowState` | `Session` | What can the workflow do next? | Rebuilt session/history state |
| `ExecutionLifecycle` | `ExecutionRegistry` and lifecycle events | Is one process currently mutating the session? | `ExecutionStarted` → `ExecutionFinished` |
| `ExecutionStatus` on messages and parts | transcript projection | Is this particular artifact still being constructed? | checkpoints followed by terminal history/events |

`blocked` and `tool_pending` are durable workflow facts. They do not mean that a task is running. A provider request is represented only by an active execution and its phase; it is never persisted as resumable work. Likewise, an in-progress message is not evidence that a worker still exists.

An execution and its canonical assistant reply have the same interactive
continuation boundary. After durably publishing an Operation-owned permission
request or a UserInput Activity, the execution moves to
`awaiting_interaction` and remains the single active writer. A durable reply
wakes that execution; it never registers a continuation execution. Only the
canonical reply boundary owns terminalization of all of its remaining
Activities.

## Single-writer boundary

`SessionManager::execute_registered` is the only lifecycle boundary for commands that may run a model or compaction. It performs this sequence:

1. Acquire the session's exclusive slot in `ExecutionRegistry`.
2. Persist `ExecutionStarted`.
3. Run the command in a joined task.
4. Convert success, cancellation, error, or panic into one `ExecutionOutcome`.
5. Persist `ExecutionFinished`.
6. Release the exact registry owner that was acquired in step 1.

A second execution for the same session fails with `ExecutionAlreadyActive`. It never replaces or implicitly cancels the current writer. Cleanup is identity-checked, so an old task cannot unregister a newer task.

Do not manually combine `register`, `begin_execution`, and `finish_execution` in a new entry point. That recreates early-return and panic leaks.

## Cancellation ownership

The processor is the sole observer responsible for converting a cancellation token into terminal transcript state. Callers must await it; they must not race the processor in an outer `select!` and drop its future.

Cancellation is cooperative for 500 ms. Provider streams, plugin hooks, tool
waits, and shell processes all observe the same token during that window. If
an adapter never yields, `ExecutionRegistry` aborts only the inner operation
task. The `execute_registered` lifecycle owner is not aborted: it joins the
cancelled task, reconciles any unmatched `RunStarted`, persists
`ExecutionFinished { outcome: cancelled }`, and only then releases the
registry slot. This bounded escalation is the recovery path for uncooperative
code; it is not a substitute for propagating the token to each resource.

On cancellation the processor:

1. stops consuming the provider stream;
2. cancels nonterminal message parts;
3. discards incomplete tool-call fragments that cannot become valid history;
4. commits `AssistantMessageFinished { status: cancelled }`;
5. commits `RunAborted { reason: user_cancelled }`;
6. returns `AppError::Cancelled`, allowing the lifecycle owner to emit `ExecutionFinished { outcome: cancelled }`.

## Interactive request lifetime and durable presentation

Interactive requests (Operation authorization and UserInput) must never block
the host indefinitely, and a shown-but-unanswered request must never be lost.

- **Bounded lifetime.** Every UserInput request carries an effective
  auto-resolution timeout. When a plugin or host API does not specify one,
  the runtime applies the default (`DEFAULT_USER_INPUT_TIMEOUT_MS`, 10
  minutes); a requested value is capped at `MAX_USER_INPUT_TIMEOUT_MS` (24
  hours). On expiry the existing `UserInputReplyKind::Timeout` path durably
  resolves the pending part and wakes the awaiting execution, so the host is
  never permanently blocked by a request nobody answered.
- **Durable presentation acknowledgement.** The `presented_at` field on a
  pending UserInput request records, on the request part itself, that a
  client has shown the request to the user. `POST
  /api/v1/sessions/{session_id}/interactive/{request_id}/present` (and the
  in-process TUI backend equivalent) performs this acknowledgement
  idempotently: the first call persists `presented_at`; later calls are
  no-ops; an unknown request id is rejected.
- **Clients reconcile against the durable field, not a volatile ledger.**
  Auto-open eligibility is `outstanding && !presented_at && !locally-seen`.
  A never-presented request still pops up after a restart or on another
  client; a presented-but-unanswered request stays visible through a
  persistent attention hint and can be reopened manually instead of
  re-prompting a modal.
- **Closing a modal must not discard the request.** ESC/close only removes
  the local "seen" guard. User input requests then fall back to the
  awaiting-input hint (and `open_pending_user_input`), while permission
  requests (which have no durable presentation field) remain auto-open
  candidates on the next sync until replied.

## Event protocol

Every execution and model run has stable typed identity:

- `ExecutionId` correlates all work performed by one public command.
- `RunId` correlates one provider round inside that execution.
- `MessageId` and part ids address transcript artifacts.

Lifecycle and terminal transcript events are persistent. High-volume deltas are ephemeral. A checkpoint may contain nonterminal state, but it is never authoritative after a terminal transition.

Checkpoints are explicit changed-part deltas. Updating one Operation, including
its authorization history, checkpoints only that part instead of rewriting
every part in the owning assistant message.

The required pairs are:

- exactly one `ExecutionStarted` followed by exactly one `ExecutionFinished`;
- every `RunStarted` followed by `RunCompleted` or `RunAborted`;
- every streamed assistant message followed by `AssistantMessageFinished` with a terminal status.

## Projection rules

The event log is the source of truth. Activity tables are rebuildable projections.

- Reads compare the projection cursor with the event store and transactionally apply every missing session event in order; structural corruption triggers a full rebuild.
- `RunAborted` terminalizes open messages and parts for its `RunId`.
- `ExecutionFinished` terminalizes execution-owned open artifacts unless a legacy completed event is replayed for a reply that still contains unresolved Operation authorization or UserInput.
- Normal interactive waiting never emits `ExecutionFinished`; the active execution remains in `awaiting_interaction` until it resumes or is cancelled.
- When no interaction is pending, a genuinely completed/failed/cancelled reply terminalizes every remaining reply-owned Activity. The reply is the cleanup boundary.
- A failed/cancelled reply cancels unanswered UserInput Activities and terminalizes Operations with unresolved authorization, so the UI never exposes a request that has no resumable owner.
- A delayed nonterminal checkpoint cannot reopen a terminal message.

Event persistence happens before projection application, and durable history is broadcast to live clients only after the projection barrier succeeds. If projection application fails, the next read detects the cursor gap and deterministically catches up from the durable event log. A later event can never advance the projection cursor past an earlier unapplied event.

## Restart recovery

Normal projection rebuild does not infer that an execution was interrupted; a rebuild can occur while the process is healthy. Only explicit startup recovery scans for unmatched lifecycle starts.

At startup, unmatched executions and runs receive terminal events with `process_restart`. Those events flow through the same projection rules as ordinary failures, so no historical message remains pending or in progress after recovery.

## UI contract

The API returns `workflow_state` and optional `active_execution` separately. The UI follows these rules:

- show an execution spinner only when `active_execution` exists;
- enable cancellation only when `active_execution` exists;
- never infer execution liveness from the last assistant message;
- preserve each assistant message as its own resource instead of collapsing consecutive rounds and borrowing the last round's status;
- never clear `active_execution` on `AssistantMessageFinished`, because tools or another model round may still follow;
- derive pending permissions from each Operation's unresolved authorization records; do not render Permission as an independent Activity;
- expose every pending interactive request independently; a provider tool batch publishes all Operation authorization requests before waiting;
- treat an individual interaction reply as a durable decision, not as a user message or a new assistant reply; the final decision in a batch wakes the existing canonical reply execution;
- treat lifecycle events as refresh triggers and fetch authoritative workflow state after execution completion.

Optimistic event reduction may improve responsiveness, but it must preserve these ownership rules.

Clients that fetch execution state and messages separately read execution state first, then the transcript. `ExecutionFinished` advances the durable projection before the registry becomes inactive, so an inactive state fences a terminal transcript. Clients also compare `latest_event_seq` before applying a response and discard any response older than an event already reduced locally.

## Review checklist

Any change to session execution should answer all of the following:

- Does it enter through `execute_registered`?
- Can any `?`, panic, join failure, cancellation, or provider error bypass terminal publication?
- Does every mutable artifact carry its execution/run identity?
- Can a late event move terminal state back to pending or in progress?
- Does restart recovery produce durable terminal events instead of mutating a projection ad hoc?
- Does the UI derive liveness only from `active_execution`?
- Is the event log sufficient to rebuild the same terminal transcript from an empty projection?
- Can every interactive request resolve within a bounded timeout, with the
  default/cap enforced at the runtime layer rather than by each caller?
- Is a shown-but-unanswered request recoverable from the durable
  `presented_at` acknowledgement instead of a volatile client ledger?
