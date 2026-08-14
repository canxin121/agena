# Durable background-operation state machine

Status: accepted and implemented in schema v9 (2026-08-14).

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
- optional launch run/tool-part references (absent only for scheduled
  deliveries);
- one `kind`: `shell`, `task`, `monitor`, or `scheduled_delivery`;
- one stable external runtime identity;
- one lifecycle `phase`;
- outcome/failure values;
- the greatest producer event sequence observed;
- a renewable runtime owner/lease;
- one monotonic optimistic `revision`;
- creation, update, and terminal timestamps.

The store rejects a second operation for the same launch tool and a second
operation for the same `(kind, external_id)`.

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
3. create a new `Runtime`-role `runtime_ingress` run at the event's real
   arrival position;
4. append one `system_notification` part beneath that run;
5. insert one `Pending` delivery row.

Notifications are never appended retroactively to the launching assistant
run. They therefore cannot reorder history or masquerade as assistant reply
text. Provider projection maps the explicit Runtime input to system context,
while the transcript retains its distinct notification identity.

### Assistant-run handoff at ingress

A Runtime ingress is a hard conversation boundary. At a provider/tool-loop
boundary, the session manager terminalizes the preceding assistant run before
opening the assistant run that answers the notification. If the preceding run
still owns a pending tool or interaction, the handoff remains execution-local
and deferred until those children settle; it never becomes another persisted
lifecycle flag. The manager must not discard its local run ID first.

This rule covers user steer and Runtime ingress uniformly. It prevents the
fast-completion ordering in which the notification response finishes while
the launch assistant marker remains `Pending`. User-input and Runtime-ingress
markers themselves are committed terminal receipts; only actual assistant
execution and pending interactions gate the derived session state.

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
`Consumed` only after the answering assistant run commits successfully. Cursor
observation alone is not success: a mid-turn dispatcher waits for the active
execution to end and verifies a completed assistant `continue` run after the
Runtime ingress. A wake failure stores its diagnostic and returns the row to
`Pending`.

If the response commits and the dispatcher crashes before the delivery consume
transaction, recovery derives that response from the append-only transcript
and consumes the delivery without invoking the model again. This closes both
sides of the handoff: a crash before response cannot lose the wake, and a crash
after response cannot duplicate it. No second `responded` flag is stored.

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

## Crash/restart behavior

| Crash point | Durable state | Recovery |
|---|---|---|
| Before operation insert | No side effect allowed yet | Tool attempt may retry normally |
| After `LaunchRequested`/`Launching`, before adapter | Intent + bounded lease | Expired intent becomes terminal; no invisible work |
| After `Running`, before process/child creation | Reserved identity + lease | Registry/child lookup after lease expiry detects absence and interrupts |
| Process finishes before tool receipt | Identity already indexed | Completion settles the aggregate; receipt handoff observes terminal state |
| Event transaction commits before wake | Notification part + `Pending` delivery | Startup/maintenance dispatcher claims and wakes it |
| Dispatcher crashes while claimed | Expiring `Claimed` row | A later dispatcher reclaims it |
| Active loop observes ingress, then fails before response | Runtime ingress + no later completed assistant response | Dispatcher/recovery starts a fresh wake; delivery remains unconsumed |
| Assistant response commits before delivery consume | Runtime ingress + later completed assistant `continue` run + claimed delivery | Recovery consumes without a duplicate model wake |
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
`record_background_event` owns the operation + Runtime ingress + notification
part + delivery transaction. Adding a second ad-hoc notified flag, part guard,
observer correlation map, or tool-specific lifecycle column is prohibited.

The design principle is:

> Everything observable is a part. Operational control state is normalized
> once and projects into parts, activities, and model input.

## Schema v8 migration

Schema v9 adds the two background tables without rewriting transcript rows.
The v8 -> v9 migration backfills legacy tool markers that contain both a
recognized kind and external ID using `bg_<session>_<tool-part>` as the stable
operation ID. Legacy corrupt markers without an external ID cannot be
reconstructed; they remain historical transcript data and are not fabricated.

## Required regression coverage

The permanent suite covers:

- v8 -> v9 migration and marker backfill;
- validated phase transitions and terminal immutability;
- completed launch receipt and absence of synthetic guards;
- pre-launch deterministic task/process identity;
- Runtime ingress role and chronological ordering;
- terminal/event idempotency;
- concurrent and out-of-order monitor events;
- exclusive delivery claims, expiry, retry, and consume;
- crash-after-commit delivery recovery;
- response-commit-before-delivery-consume recovery without a duplicate model
  wake;
- observer-free task settlement and restart interruption;
- expired process-owner interruption;
- scheduled delivery in an empty session and concurrent redelivery;
- Runtime ingress during a tool-calling turn, including terminal handoff of the
  preceding assistant run and a final `Ready` session;
- terminal user-input receipts that do not act as execution-liveness guards;
- SQLite and in-memory backend conformance.
