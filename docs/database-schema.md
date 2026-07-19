# Current database schema

This is Agena's one current development database design. It separates
authoritative event history, session provenance, mutable session state, and
disposable activity projections so that each write path has one owner and one
set of invariants.

## Development reset policy

There is no database version marker and there are no migration scripts.
Initialization only creates the current tables, indexes, and invariant
triggers with `IF NOT EXISTS`.

- A new database is created directly in the current format.
- Initialization never inspects `PRAGMA user_version`.
- Initialization never migrates, alters, drops, or automatically rebuilds an
  existing database.
- No legacy columns, payload adapters, data repair, or version branches are
  kept in runtime code.
- If a development database does not match the current source definitions,
  delete that database and let Agena create a new one.

Every SQLite connection used by Agena must have foreign-key enforcement
enabled. Schema initialization fails instead of running without it.

## Ownership boundaries

### `agena_sessions`

Owns the session's stable hierarchy projection and mutable presentation/runtime
metadata:

- workspace, title and optimistic-lock version;
- immutable `parent_id`, `root_id`, and `depth`;
- creation lifecycle (`creating`, `ready`, or `failed`);
- opaque runtime state;
- creation and update timestamps.

The hierarchy fields are computed once at creation. They cannot be changed by
the update API or by SQL. Moving a session is not an operation: callers create
a child, fork, or rewind instead.

### `agena_session_lineage`

Owns the meaning of every non-root hierarchy edge:

- `child`: an empty child created explicitly by a caller;
- `fork`: copied history through a source event boundary;
- `rewind`: copied history before a selected source message;
- `subagent`: a delegated task session.

Fork and rewind rows persist their source cutoff and optional selected message.
Subagent rows persist task identity and the authoritative delegated-task
lifecycle. Roots have no lineage row. Titles are never used to infer lineage.

Lineage provenance is immutable. Subagent lifecycle fields may change through
the dedicated subtask operation; unrelated runtime writes cannot overwrite
them. Session and lineage rows are created as one transactionally atomic
aggregate; a failed lineage insert cannot leave a child session behind.

`relation_kind` is the only source of subagent identity in core/domain code.
Any `is_subagent` flag exposed by a presentation DTO is derived at the output
boundary and is never persisted as independent state.

### `agena_events`

Is the authoritative, append-only domain history. Event UUIDs, global
sequences, and per-session sequences are unique. Session-scoped events require
both `session_id` and `seq_session`; non-session events require neither. When
the optional denormalized `workspace_id` is present on a session event, the
database verifies that it matches the owning session.

Events belong to their session through a foreign key. Deleting a session is an
explicit request to delete its event history as well; the current schema does not mix
hard deletion with an audit-retention interpretation.

### `agena_activity_messages`

Is a disposable read model projected from the event log. `message_id` is a
globally unique storage identity and permanently belongs to one session.
Session ownership, turn identity, role, and creation time cannot be changed by
an upsert or direct SQL update.

A fork or rewind creates new message IDs. It never moves or aliases source
message rows.

### `agena_activity_parts`

Is a disposable child read model. `part_id` is globally unique and permanently
belongs to one message. The table intentionally has no `session_id`: session
ownership is derived through `part.message_id -> message.session_id`, removing
the possibility of contradictory ownership columns.

A fork or rewind creates new part IDs and rewrites all copied message/part
references before projection.

### `agena_activity_projection_states`

Tracks the last projected global sequence. It does not carry a projector
generation. Projection code changes replace the current implementation
directly; developers reset an incompatible database instead of migrating a
read model across versions.

## Branch creation lifecycle

Fork and rewind creation is intentionally staged:

1. Insert the child and its immutable lineage with `lifecycle_state=creating`.
2. Allocate fresh global message and part identities.
3. Rewrite copied entity references and event-owned session references.
4. Append copied persistent events without broadcasting them as new activity.
5. Build the activity projection and remap a retained prompt-window checkpoint.
6. Transition the child to `ready` and publish it through normal caches/lists.

If replay or projection fails, the row transitions to `failed`, records the
error, and is removed from cache. Ordinary list/get/load paths expose only
`ready` sessions. Lifecycle transitions are one-way:

```text
creating -> ready
creating -> failed
```

## Deletion

There is one hard-delete path. Deleting a session deletes the full descendant
ownership graph through foreign keys:

```text
session
|- descendant sessions
|- lineage
|- events
|- permission rules
|- activity messages
|  `- activity parts
`- projection state
```

API handlers must call the session repository deletion function; they must not
issue an entity-level delete directly.

## API contract

The API exposes `relation_kind`, `lifecycle_state`, source cutoff, and source
message metadata. Session update requests contain only mutable fields (currently
the title); they cannot carry `parent_id`.

The development protocol remains fixed at `1` (`1.0`). Server and clients are
changed together against the one current contract. Incompatible session
semantics replace that contract directly; they do not create protocol or
database generations.

## Future evolution

Physical event copying remains the branch implementation because it keeps
reads, pagination, provider replay, deletion, and export straightforward. Add
structural history sharing only after measurements show branch-copy volume or
latency is a material bottleneck.

Runtime state may later move into independently locked rows by write owner
(workflow, prompt/cache state, and execution configuration). Do that when
concurrent writers need independent commits; do not normalize provider-opaque
JSON merely for table shape.
