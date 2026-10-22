# Error Parts and retries

Execution failures are durable server-owned transcript Parts. They use the
ordinary storage facade, Part IDs, revisions, membership, live notifications,
transcript paging, folding, navigation, copy, and export paths.

Part identity is separate from timeline position. Read-time projections of
failed provider rounds use the round's completion time, and server snapshots,
Web reply folding, and TUI snapshot merging order content by creation time,
owning run, and Part ID. A synthetic error ID must not move an earlier attempt's
failure behind the content of a later attempt. The later attempt's run marker
continues to own the reply state.

- A model failure creates one `error` Part owned by its original run. Repeated
  attempts and terminal cleanup update that Part. Its Part ID and public
  failure correlation ID stay stable.
- An internal retry reopens its failed run in place, clears the marker's
  `finished_at_ms` and `abort_reason`, and retains its existing children.
  Completed and cancelled runs do not reopen through this transition.
- A checkpoint can require a new run in the same reply. Failure recording
  resolves the existing error by `reply_id`; it preserves the original error's
  ownership instead of allocating another error for the retry.
- A fork's inherited history remains read-only. Error recovery and retry
  updates are limited to Parts owned by the current session, even when a
  branch continues the same reply ID.
- Provider retry progress lives in the error Part's typed `retry` value:
  `attempt`, `max_retries`, optional `next_at_ms`, `phase` (`backoff`,
  `requesting`, or `streaming`), optional `started_at_ms`, and `outcome`
  (`retrying`, `recovered`, `failed`, or `cancelled`). `attempt` is the upcoming
  retry number within its retry budget; the initial request is excluded.
  A provider bookkeeping event advances the same Part to `requesting` after
  backoff and before the next provider call. First content advances it once
  to `streaming`; subsequent tokens do not rewrite retry progress. Finishing
  removes its phase and timing fields. Only reviewed user-facing messages are
  persisted.
- Recovery leaves the error Part in history. The run can be `completed` while
  its historical error Part remains `failed` and its retry outcome is
  `recovered`. The folded reply status follows the latest run marker.
- Tool failures update the existing tool Part. A failed compaction that already
  owns its structured problem likewise remains on its compaction Part; cleanup
  does not add a second error or execution envelope for it.
- Startup, prompt preparation, provider streaming, orchestration failures,
  unexpected task exits, and interrupted-process recovery all persist failures
  on the server. Reopening an inactive session repairs older failed markers
  missing an error record. Concurrent repair and repeated cleanup are
  serialized per session and do not duplicate Parts.
- Cancellation does not manufacture a failure. If it interrupts retries whose
  error Part already exists, that history remains and retry outcome becomes
  `cancelled`. Rejected input before submission remains an API rejection.

Web and TUI render loaded Parts. They do not turn message metadata, session
error events, missing paged content, or a remembered error into transcript
content. In particular, a later successful run never requires a client cache
to keep an earlier failure visible. Generic loading, connection, and command
feedback remain separate from execution history.

Both clients show the reason and retry progress in the ordinary Part headline,
in collapsed and expanded views. Waiting shows the retry count, a live countdown,
and the planned local time. An active request shows its phase and elapsed time;
recovery, exhausted retries, early failure, and cancellation have distinct final
labels. A past deadline without a server start transition means “waiting for
retry to start,” never a fabricated running state or a permanent zero countdown.
Older stored progress without phase fields remains readable. Header colors and
icons follow retry progress while the historical Part's stored state stays
`failed`.

Web clocks stop when a pane/document is hidden or a retry ends, and catch up from
absolute timestamps on return. TUI refreshes active retry entries once per second
while retaining static entries' rendered blocks. Both retain the Part's ordinary
disclosure, keyboard navigation, and identity; progress adds no separate banner
or nested card.

Regression coverage includes concurrent failure writes, startup and streaming
failures, successful retries, repeated failure, checkpoint ownership, task
panics, restart repair, failed hosted image operations, normal Web Part DOM
identity, stale live delivery, overlapping pages, reconnection, and TUI
rendering/cache identity. Processor result snapshots include the latest error
row alongside ordinary content, including when its original owner predates a
checkpoint; returned Sessions and durable storage therefore agree immediately.
