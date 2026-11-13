# Operator Bot

Bots are persistent named identities with independent personalities and durable
conversations. Web and TUI use the same server registry. Each Bot has a stable
UUID separate from its current session ID; different clients can open different
Bots without changing a global selection. A Bot's own native session model
selection can use any configured provider, including a configured local model.
Names and persona fields never grant filesystem, network, or approval authority.

`POST /api/v1/bot/session` resolves the oldest active Bot and returns
`{"session_id": N, "bot_id": "UUID", "name": "Agena"}`. It takes no body.
`GET /api/v1/bots?include_archived=false` lists up to 256 registry records in
creation order, with `default_bot_id`. The authenticated registry API also offers:

- `POST /api/v1/bots`: create with `request_key`, `name`, optional `persona`.
- `GET /api/v1/bots/{id}`: inspect the exact profile, including archived records.
- `PUT /api/v1/bots/{id}`: update explicit `name`/`persona`, guarded by
  `expected_revision` and `request_key`.
- `POST /api/v1/bots/{id}/session`: open or recover only that active Bot.
- `POST /api/v1/bots/{id}/archive`: archive with revision and request key while
  the source has no active/queued execution, pending native interaction, active
  direct Subagent, or native background activity.

A persona carries exactly one field, `instructions` (up to 256 KiB). Active
names are trimmed and Unicode case-folded for uniqueness against other active
Bots, and reject empty/control/overlong input.

Bots live in their own `agena_bots` table. Every conversation that belongs to a
Bot is an ordinary `agena_sessions` row whose configuration carries
`owner.bot` and `owner.surface`, projected into indexed `owner_bot_id` /
`owner_surface` / `owner_scope_key` columns. A Bot therefore has no single bound
conversation: `POST /api/v1/bots/{id}/session` returns its most recently used
local root and creates a fresh one when that conversation was deleted.
Ownership is immutable after creation, so an archived conversation stays
read-only because its row still names the Bot.

All writes are single compare-and-set statements: `PUT` and archive carry
`expected_revision`, and each `request_key` has a stored receipt, so a stale
edit cannot overwrite a newer one and a retried request cannot repeat an effect.
Repeated opens preserve human-edited conversation
titles and model settings. Deleting a conversation through ordinary session
management recovers a fresh conversation for the same Bot UUID and persona;
other Bots keep their own conversations. Reopening while another operation waits
for a native approval remains possible.

Archiving preserves history and makes the Bot profile read-only. It immediately
revokes Bot management authority. The default becomes the oldest remaining active
Bot; archiving the last Bot leaves no default, and opening returns an actionable
not-found response until a user creates a Bot. All active, archived, and replaced
Bot session IDs and their descendants remain excluded from management targets
and discovery by every Bot.

Bot persona replaces workspace presentation in its model context. Conversation
history and model selection are independent. Project memory, reminder permission
and contact budgets keep their existing workspace scope; Bot names do not create
separate project memory stores. Ordinary project chats retain their workspace
assistant preferences. Bot settings edit only the selected Bot's presentation.

The bundled `bot.sessions` tool resolves project names/paths, shows registered
project details, and lists ordinary conversations globally or within a project.
Session filters include title, favorite, pinned, native state and parent. It
returns real IDs and `/chat?sessionId=N` links so users need not find IDs.
Continue relevant existing work with `send`, or create an ordinary root and send
its self-contained instruction. Read/read_text and status return bounded,
paged evidence and pending decision links. Accepted, queued, running, awaiting
human input and terminal outcomes are distinct; a queued instruction is not
proof of completed work.

Metadata actions rename or explicitly set favorite/pinned with an observed
version. Fork creates an ordinary visible native Fork in the same project,
inheriting the transcript through a stable boundary, model and permission
constraints; an optional title commits with the branch. Models reflects actual
configured native providers; configure preserves omitted fields and compares
the observed metadata version. Queue listing and explicit pause, text edit and
remove use native control revisions and execution/generation guards. Text edits
preserve attachments. Pausing all conversation work first fences the native
queue, then stops only the observed active execution; queued inputs remain
unless removal was requested. Resume continues that queue without resubmission.
Export returns the existing authenticated streaming JSONL
route, `/api/v1/sessions/{session_id}/export`, without filesystem writes.

Delete soft-deletes exactly one idle conversation after a native human input
card in Bot names its workspace path/ID, title and session ID. Metadata version,
control version, transcript entry count, pending/running/queued work and the
latest authorizing human turn are rechecked atomically after confirmation.
Changed targets or a newer human turn require fresh confirmation. Project files
and directories are never deleted. Project registration only records an
existing directory; ordinary filesystem tools retain their permission checks.

The host derives the caller from its live callback capability and checks it
against active registry membership. Ordinary project sessions, Subagents,
side conversations and channel sessions cannot acquire
cross-project authority by supplying another ID. Targets must be ordinary
project conversations, including human-created children, forks and rewinds.
Their retained ancestry is checked to preserve isolation, including after an
ancestor is soft-deleted. An ordinary surviving fork remains available after its
source is deleted; deleting a channel/Subagent ancestor does not erase inherited
restrictions. Isolated channel workspaces
are also excluded from project registration, listing and creation. The tool cannot approve permission cards,
change permissions, or read channel conversations.

Every write requires an exact quotation from the current direct user's message
and a stable operation key. The existing live source guard rejects tool output,
background notices and control continuations as new user authority. Persisted
operation intents reject reuse of a key with changed input; kernel command
identities protect creation, rename and instruction submission across retries.
Rename and organize compare the observed metadata version inside the kernel transaction. Fork boundaries and confirmed deletion guards persist with the intent; native command receipts remain replayable after a target is completed or deleted.
Stop uses the observed execution ID and native execution generation, so it
cannot cancel a newer execution after a takeover or retry.

Routed work executes in normal visible project sessions. New roots start with
normal project permission and model resolution; existing roots preserve their
selected model. Instructions use the existing transactional system-delivery and
serialized execution queue, with `bot_instruction` provenance carrying the Bot
session and real user-message IDs. They are system continuations, never
fabricated user messages. The user can send messages, take over, select a model,
inspect Subagents and resolve permission cards through the normal interfaces.
Project execution uses existing tools, native execution identities and Subagents. Subagent results resume the existing
project through native system continuations even before a human has sent a
message in that project; they do not fabricate user authority.

Ordinary project guidance now directs work in the current conversation. The operator Bot uses the same normal project session operations as a human.

Focused checks (all fixtures are isolated):

```sh
cargo check --offline --locked -p agena-api-server -p agena-client -p agena-bundled-plugins
cargo test --offline --locked -p agena-runtime-session session::manager::bot::tests --lib
cargo test --offline --locked -p agena --test system scenarios::bot -- --nocapture
```
