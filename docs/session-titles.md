# Automatic session titles and auxiliary models

The server names new sessions in a separate, tool-free model request. The main
assistant does not need to remember a naming instruction. Web and TUI sessions
created with a generated placeholder opt in automatically, including sessions
created from TUI's project hub. JSON-RPC sessions without an explicit title
also opt in. Explicit names from API clients stay fixed unless the client opts in.

The title worker runs asynchronously, so naming does not hold up a submitted
message or the assistant's response. The first user message can generate a
title while the first response is running. After three user messages have
settled, the worker checks the name again. Later checks require both the
configured number of new user messages and the cooldown. Tool calls and model
iterations do not count as user messages. A check can keep the current title;
normal investigation, implementation, and testing within a task should not
cause repeated renames.

## Model selection

The **Models** settings page in Web, and the **Models & providers** settings in
TUI, expose an **Auxiliary model** selection at global and workspace scope. It
is shared by short background activities; session naming is its first user.
An unset workspace value inherits the global value. If neither layer sets a
model, each task uses its session's effective model and inference modes.

For example, using an existing configured provider and enabled adapter:

```json
{
  "runtime": {
    "auxiliary_model": {
      "provider": "openai",
      "adapter": "openai_chat_completions",
      "model": "gpt-4.1-mini"
    }
  },
  "session": {
    "auto_title": {
      "enabled": true,
      "message_interval": 5,
      "cooldown_seconds": 300
    }
  }
}
```

The auxiliary selection also accepts `thinking_mode`, `speed_mode`, and
`verbosity`. A selection is replaced as a unit when a higher-priority layer
overrides it; inference modes from a different model are not inherited. The
provider and model are required, and the provider and optional adapter must
exist and be enabled. Delete the selection to use the inherited value again.
In the global layer, deleting it restores the session-model fallback.

Auxiliary calls use the selected provider's normal credentials and network
configuration, with a 30-second deadline and a shared limit of four concurrent
requests per workspace runtime. They have their own bounded input, no tools or
native conversation replay, and do not add messages to the main transcript or
change its model.

## Naming policy

The **Runtime & session** settings expose the title policy. Defaults are
enabled, five new user messages between later checks, and a five-minute
cooldown. `message_interval` must be positive; `cooldown_seconds: 0` removes
the time condition. The first name and the third-message calibration are
exempt from the later-check cooldown. Checks occur on conversation events;
the server does not poll idle sessions solely to rename them.

Title provenance and successful-check progress are stored with the session.
Invalid output, provider errors, and timeouts leave the current name and
progress untouched, allowing a later event to retry. Queued work is coalesced
per session. Only one naming request per session runs at a time, including
when a new user message arrives during generation.

## Manual names and API clients

Assigning a name through the UI, API, or an explicit user request to
`session.rename` locks that title. This applies even when the supplied name
equals the current name. A late background result cannot overwrite it.
Sessions saved by earlier versions have no provenance and are treated as
manually named, preserving existing names.

API clients can opt a new session into automatic naming:

```http
POST /api/v1/sessions
Content-Type: application/json

{"workspace_id":1,"title":"New session","auto_title":true}
```

To resume automatic naming for an existing session, explicitly enable it:

```http
PUT /api/v1/sessions/123
Content-Type: application/json

{"auto_title":true}
```

The next conversation event triggers its initial check. `{"auto_title":false}`
locks the current title. Assigning `title` without `auto_title` always locks
it. Agent child sessions retain their assigned names. Forks and rewinds of
automatically named sessions start a fresh naming schedule; branches of
manually named sessions preserve the manual lock.

JSON-RPC's `session/create` accepts an optional `auto_title` boolean. When it
is omitted, a supplied title stays manual and an omitted title is automatic.

The database applies background results with a conditional metadata update.
It rechecks the metadata version, branch membership, user-message count, and
the latest user message's identity and revision in the same transaction as
the title write. Edits, new messages, manual names, deletion, or superseded
metadata therefore reject stale results. The existing session change feed
updates connected clients.

## Verification

Focused checks for configuration, scheduling, output validation, and real
SQLite persistence/concurrency behavior:

```sh
cargo test --locked -p agena-runtime-config --test auxiliary_model
cargo test --locked -p agena-storage-sqlite --test automatic_titles
cargo test --locked -p agena-runtime-session --lib session::manager::titles::tests
```
