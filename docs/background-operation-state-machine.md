# Durable background-operation state machine

Status: accepted and implemented in schema v10 (2026-08-15).

This is the authoritative design for background shell processes, delegated
tasks, continuous monitors, and scheduled deliveries. It supersedes the
background lifecycle and notification-persistence sections of
`fusion-design.md`.

## Decision

Agena stores one `BackgroundOperation` aggregate for every launched background
operation. That aggregate is the only authoritative operational lifecycle.
Tool parts, child-session metadata, the process registry, the activity panel,
observer events, and model notifications are projections or liveness hints;
none may independently decide the operation's state.

This is deliberately not one session-wide mega-enum. A session may own many
independent operations at once, so collapsing them into a session state would
lose identity and create invalid combinations. Continuity comes from one
aggregate type, one transition graph, one revision, and one store API per
operation.

The aggregate is stored in `agena_background_operations`. Observable events
and their delivery handoff are stored in `agena_background_deliveries` and
projected into normal transcript parts.

## Aggregate boundary

Each operation owns:

- a stable internal `operation_id`;
- the owning `session_id`;
- paired optional launch run/tool-part references (present for every
  AI-created operation, including schedules; absent only for launch-less
  host/external scheduled deliveries);
- one `kind`: `shell`, `task`, `monitor`, or `scheduled_delivery`;
- one stable external runtime identity;
- one lifecycle `phase`;
- outcome/failure values;
- the greatest producer event sequence observed;
- a renewable runtime owner/lease;
- one monotonic optimistic `revision`;
- creation, update, and terminal timestamps.

The store rejects a second non-scheduled operation for the same launch tool
and a second operation for the same `(kind, external_id)`. Recurring schedule
fires are separate delivery operations that intentionally share the creating
`cron.create` part and remain unique by delivery key.

## Operation state graph

```text
LaunchRequested
       |
       v
   Launching
       |
       v
    Running -----------------------------+
       |                                  |
       +--> Completed                     |
       +--> Failed                        |
       +--> Cancelled                     |
       +--> TimedOut                      |
       +--> Interrupted                   |
                                          |
Launching may also reach a terminal state-+
when an adapter fails or a very fast operation completes during handoff.
```

Terminal phases cannot transition again. Replaying the same terminal event is
handled by event idempotency and returns the existing delivery; it does not
rewrite terminal state.

`Running -> Running` is permitted only as a revision-checked ownership-lease
renewal. A monitor event keeps the phase unchanged and advances the aggregate
revision/cursor atomically.

## Launch protocol

The foreground tool call and background work have different lifecycles:

1. Permission and input validation finish.
2. The manager allocates a stable external identity before any external side
   effect. Task IDs omitted by the caller are written back into the durable
   invocation. Shell/monitor process IDs are derived from `(session_id,
   call_id)`.
3. `LaunchRequested` is inserted idempotently.
4. The manager transitions through `Launching` to `Running`, binding the
   external identity and a short launch lease.
5. Only then may the adapter create the process/child session.
6. The adapter returns a launch receipt. Its kind and external ID must exactly
   match the reserved identity; otherwise the operation becomes `Failed`.
7. The tool part becomes `Completed` immediately. It is an immutable launch
   receipt, not the lifecycle owner. No empty `tool_result` guard is created.
8. The short lease becomes a renewable runtime ownership lease.

Reserving identity before step 5 removes the “operation completed before the
manager registered it” race. The process registry also inserts a reserved ID
before starting its worker; replaying the same ID returns the existing process
instead of launching a duplicate.

## Events, transcript order, and delivery

Every externally visible background event is recorded by one store
transaction:

1. deduplicate `(operation_id, event_key)`;
2. validate/advance the operation;
3. if `launch_run_id` exists, append one Assistant-role
   `system_notification` part beneath that exact launch run;
4. otherwise (scheduled/external ingress), create a terminal Runtime-role
   `runtime_ingress` run and append the notification beneath it;
5. insert one `Pending` delivery row.

An AI-launched shell, task, monitor, or scheduled hook therefore keeps the identity of the
assistant turn that caused it and creates no synthetic System/Runtime message.
The part is still rich `system_notification` activity rather than assistant
reply text: UI projection renders a Hook row, and provider projection maps its
body to a typed system-context input. A host/external schedule with no
assistant launch provenance remains an explicit chronological Runtime ingress
instead of inventing an AI author.

`cron.create` persists `(session_id, run_id, tool_part_id, call_id)` inside the
canonical scheduler job. Every recurring fire copies the paired run/tool
references into its `scheduled_delivery` aggregate. The provenance is chosen
at creation time and never inferred from the latest transcript row, so restart,
compaction, retries, and later assistant turns cannot redirect a fire.

Presentation and provider chronology deliberately use different projections
of the same part. Full-history grouping attaches the hook to its original turn
for UI identity. Provider projection removes Assistant notification parts from
retroactive run grouping and inserts each one immediately before the first
provider round whose durable input receipt lists it; an unhandled hook stays at
the prompt tail. This keeps the request prefix stable as it grows from
`[..., hook]` to `[..., hook, response]` and prevents a late hook from appearing
before user/assistant work that had already happened. If compaction removes the
old launch marker from the active-window slice, the notification's
`delivery_protocol = provider_round_v1` marker pins the still-unhandled part
after the checkpoint. It is unpinned only when a completed provider-round
receipt names its part ID.

### Part-boundary handoff while the AI is active

An AI-owned notification is not a hard conversation boundary and never
terminalizes the current assistant run. If it arrives during a provider stream
or tool part, the dispatcher queues one steer signal. `drain_steer_input` is
only entered after the current provider/tool part returns to the stable-loop
boundary; it reloads the already-persisted hook and requests the next provider
round on the same assistant turn. Multiple hooks queued before that boundary
are reloaded and acknowledged together.

User input and launch-less Runtime ingress remain real external boundaries:
the manager closes the preceding assistant run only after all of its child
parts are terminal, then opens the answering run. This distinction prevents
the previous `Hook -> synthetic resume reply -> Hook` fragmentation without
weakening user-turn ordering.

Monitor event keys are `event:<producer-seq>`. Events are individually
idempotent. Concurrent/out-of-order events are all retained; `last_event_seq`
stores the greatest sequence observed rather than rejecting a late lower
sequence. The terminal key is `terminal`, so monitor events cannot consume or
hide terminal completion.

## Delivery state graph

```text
Pending --> Claimed --> Consumed
   ^           |
   +-----------+
      wake failure or expired claim
```

Claim is an atomic compare-and-set with a bounded expiry and attempt counter.
Only the claimant may consume or release it. A notification becomes
`Consumed` only after a provider round that actually received it commits
successfully. Cursor observation alone is not success. Every successful round
stores `input_notification_part_ids` beside its existing `part_ids` and
provider replay state on the assistant run marker. The dispatcher waits for
the active execution to end and verifies that exact durable input receipt. A
wake failure stores its diagnostic and returns the row to `Pending`.

If the response commits and the dispatcher crashes before the delivery consume
transaction, recovery derives that response from the transcript's provider
round records and consumes the delivery without invoking the model again. The
exact receipt also prevents output from a provider request that started before
the hook arrived from being misclassified as its response. This closes both
sides of the handoff: a crash before response cannot lose the wake, and a crash
after response cannot duplicate it. No second mutable `responded` flag is
stored.

Startup and periodic maintenance scan pending/expired deliveries. Therefore a
crash after the event transaction commits but before wake cannot silently lose
the notification.

## Ownership and reconciliation

The operation owner/lease is a cross-process liveness hint:

- a runtime renews only operations it can prove are alive in its child
  execution registry or process registry;
- monitor events preserve a live lease; terminal events clear it;
- another process does not reconcile an operation while its lease is live;
- after expiry, task reconciliation checks durable child-session status and
  child execution leases;
- after expiry, shell/monitor reconciliation checks the local process
  registry;
- work that cannot be proven alive becomes `Interrupted` and creates a normal
  durable notification/delivery instead of remaining `Running` forever.

The session observer bus still triggers low-latency task settlement, but
startup/maintenance reconciliation repeats the decision from durable rows.
The observer bus and activity registry are never correctness dependencies.

A delegated-task timer is also authoritative over cleanup results. Once the
timer wins, cancelling the child commonly returns `Cancelled`; that value is
cleanup mechanics, not the task outcome. The child is immediately committed
as `TimedOut` with a non-null `subtask.timeout` failure containing task ID,
description, and timeout duration. It must never remain `Running` for lease
reconciliation to downgrade later to `Interrupted`.

## Crash/restart behavior

| Crash point | Durable state | Recovery |
|---|---|---|
| Before operation insert | No side effect allowed yet | Tool attempt may retry normally |
| After `LaunchRequested`/`Launching`, before adapter | Intent + bounded lease | Expired intent becomes terminal; no invisible work |
| After `Running`, before process/child creation | Reserved identity + lease | Registry/child lookup after lease expiry detects absence and interrupts |
| Process finishes before tool receipt | Identity already indexed | Completion settles the aggregate; receipt handoff observes terminal state |
| Event transaction commits before wake | Notification part + `Pending` delivery | Startup/maintenance dispatcher claims and wakes it |
| Dispatcher crashes while claimed | Expiring `Claimed` row | A later dispatcher reclaims it |
| Active loop observes hook, then fails before response | Notification part + no provider-round input receipt | Dispatcher/recovery starts a fresh wake; delivery remains unconsumed |
| Assistant response commits before delivery consume | Notification part + provider round listing its part ID + claimed delivery | Recovery consumes without a duplicate model wake |
| Task observer event is dropped | Terminal child row + Running parent operation | Durable task reconciliation creates the terminal event |
| Runtime exits with Running work | Owner lease stops renewing | Another runtime waits for expiry, then reconciles registry/child state |

## Storage and code ownership

All operation writes must use these store APIs:

- `create_background_operation`;
- `transition_background_operation`;
- `record_background_event`;
- delivery claim/consume/retry APIs;
- active-operation and pending-delivery reconciliation reads.

Application/runtime code must not update the background tables directly.
`record_background_event` owns the operation + launch-run/Runtime-ingress
selection + notification part + delivery transaction. Adding a second ad-hoc
notified flag, part guard, observer correlation map, or tool-specific lifecycle
column is prohibited.

The design principle is:

> Everything observable is a part. Operational control state is normalized
> once and projects into parts, activities, and model input.

## Unified background-member projection

The Activities service is a read projection, not another lifecycle owner.
Every read merges the following authoritative rows by stable external ID:

- non-terminal `agena_background_operations` for `shell`, `monitor`, and
  `task`;
- durable scheduler jobs for `cron`/one-shot wakes;
- the bounded process-local activity registry only for log cursors, transient
  runtime maintenance, browser sessions, and terminal display history.

When a durable operation and a registry record describe the same member, the
durable phase, kind, owning session, operation ID, and launch tool-part ID win;
the registry contributes only live command/log detail. This is why a monitor
cannot silently become a shell in the UI and why a shell activity regains its
session after a restart. Scheduler jobs are projected as `cron_<job-id>` and
carry their source `cron.create` part (when present) plus `next_event_at_ms`.

`SessionExecutionResource.background_activities` contains the session-scoped,
non-terminal slice of that same Activities projection. The TUI composer footer
and Web session surface derive their per-kind counts directly from this field;
they must not keep a timer-driven or process-local counter. Activities control
actions are likewise projected from current state: ordinary active members
offer `stop`, running cron members offer `pause`/`delete`, and paused cron
members offer `resume`/`delete`. Control mutations update the durable owner
first, then publish an ephemeral activity signal for low-latency refresh.

Cron wall-clock expressions always carry an explicit IANA timezone. The
timezone is persisted with the scheduler job and reused by creation, resume,
expression update, normal advancement, and misfire rescheduling. Old jobs
without timezone metadata remain UTC for backward compatibility. Tool output
continues to use RFC 3339 instants, so a client never has to guess whether a
returned timestamp is local time.

## Schema migrations

Schema v9 adds the two background tables without rewriting transcript rows.
The v8 -> v9 migration backfills legacy tool markers that contain both a
recognized kind and external ID using `bg_<session>_<tool-part>` as the stable
operation ID. Legacy corrupt markers without an external ID cannot be
reconstructed; they remain historical transcript data and are not fabricated.

Schema v10 rebuilds the two background tables transactionally so a
`scheduled_delivery` may retain paired assistant launch references. Existing
v9 operation and delivery/outbox rows are copied without rewriting transcript
parts; launch-less scheduled rows remain valid.

## Required regression coverage

The permanent suite covers:

- v8 -> current migration and marker backfill;
- v9 -> v10 rebuild preserving pending deliveries and foreign keys;
- validated phase transitions and terminal immutability;
- completed launch receipt and absence of synthetic guards;
- pre-launch deterministic task/process identity;
- Assistant identity and launch-run ownership for every AI-launched hook,
  including recurring schedules;
- Runtime/System ingress identity for launch-less host scheduled delivery;
- delegated timeout persistence when cleanup returns `Cancelled`, including a
  specific `subtask.timeout` notification without lease reconciliation;
- terminal/event idempotency;
- concurrent and out-of-order monitor events;
- exclusive delivery claims, expiry, retry, and consume;
- crash-after-commit delivery recovery;
- response-commit-before-delivery-consume recovery without a duplicate model
  wake;
- observer-free task settlement and restart interruption;
- expired process-owner interruption;
- scheduled delivery in an empty session and concurrent redelivery;
- cron wall-clock calculation in a non-UTC IANA timezone;
- durable monitor/session/source-part and paused-cron activity projection;
- stable per-kind composer counts derived from the session resource;
- notification arrival during an active provider part, including queued
  part-boundary delivery, continuous assistant-turn identity, exact round
  receipt, and a final `Ready` session;
- terminal user-input receipts that do not act as execution-liveness guards;
- SQLite and in-memory backend conformance.
