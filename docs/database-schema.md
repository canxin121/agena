# Current database schema

This is Agena's one current development database design. It separates
authoritative event history, session provenance, the canonical conversation
graph, mutable session state, and disposable provider/history projections so
that each write path has one owner and one set of invariants.

## Development reset policy

The only accepted SQLite schema version is `8`, stored in
`PRAGMA user_version`. There are no migrations between incompatible versions.
Initialization creates the current tables, indexes, and invariant triggers
atomically and writes version `8` only for a fresh version-`0` database.

- A new version-`0` database is created directly in the current format.
- A version-`8` database is accepted as current.
- Every other version is rejected before schema mutation, whether it is older
  or newer than the application.
- Initialization never migrates, alters, drops, or automatically rebuilds an
  incompatible existing database.
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
- `creation_failure_json`, required only for `failed`, containing a structured
  semantic failure rather than a source error string;
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

### `agena_turns`

Owns the ordered conversation turns for a session. `turn_id` is the stable
domain UUID used by events, live patches, durable snapshots, and the TUI.
`turn_seq` is unique within the session; it provides canonical turn order
without deriving identity or order from provider messages.

### `agena_assistant_replies` and `agena_reply_executions`

`agena_assistant_replies` owns exactly one stable visible reply for each turn.
`agena_reply_executions` records the one originating user execution plus any
permission, user-input, continue, or compaction continuations that append to
that same reply. Executions are control/audit lifecycles only: they never
create another user turn or assistant reply. Interactive continuation resolves
the exact reply from the requesting Activity owner, never from "latest reply".

### `agena_text_segments`

Owns canonical text for either a turn input or an assistant reply. Every segment has a
stable domain UUID, immutable owner and position, and a monotonic revision.
Text is not used to encode attachment labels, Skill mentions, paste labels, or
other structured content.

### `agena_activities`

Owns all canonical structured content. Activities may belong to a turn input,
an assistant reply, another activity, or a session. The row records stable identity,
actor, typed JSON payload, state, canonical position, revision, and lifecycle
timestamps.

Text segments and activities share one position namespace within an owner.
Cross-table triggers reject a duplicate position, so an interleaved document
such as text, Skill reference, text, and directory attachment has one durable
order rather than two independently sorted collections. Owner-validation
triggers reject nonexistent or invalid owner kinds.

### `agena_model_messages` and `agena_model_message_parts`

Are disposable provider/history projections. Their integer message and part
identities support provider replay and runtime diagnostics; they are not a
public conversation API and do not define conversation-body ordering. A
projected part may carry the stable canonical `activity_id` or `segment_id`
that produced it. `awaits_user_reply` is true only for an unanswered
permission or user-input request, allowing that Activity to survive the
execution that originally requested it.

Identity, ownership, index, kind, and creation time are immutable. Part session
ownership is derived through `part.message_id -> message.session_id`, avoiding
contradictory ownership columns.

### `agena_model_projection_states`

Tracks the last global event sequence incorporated into the disposable
provider/history projection. It does not carry a projector generation.
Projection code changes replace the current implementation directly;
developers reset an incompatible database instead of migrating a read model
across versions.

## Branch creation lifecycle

Fork and rewind creation is intentionally staged:

1. Insert the child and its immutable lineage with `lifecycle_state=creating`.
2. Allocate fresh message, part, execution, run, turn, assistant-reply,
   text-segment, and activity identities.
3. Rewrite every copied entity reference, owner identity, correlation, and
   event-owned session reference before inserting any copied event.
4. Append copied persistent events without broadcasting them as new activity.
5. Build the canonical conversation graph and disposable provider/history
   projection, then remap a retained prompt-window checkpoint.
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
|- turns
|  |- assistant replies
|  |  `- reply executions
|  |- text segments
|  `- activities
|- session-owned and nested activities
|- model messages
|  `- model message parts
`- model projection state
```

API handlers must call the session repository deletion function; they must not
issue an entity-level delete directly.

## API contract

The API exposes `relation_kind`, `lifecycle_state`, source cutoff, and source
message metadata. Session update requests contain only mutable fields (currently
the title); they cannot carry `parent_id`.

The structured-failure protocol is version `2`. Server and clients are
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
