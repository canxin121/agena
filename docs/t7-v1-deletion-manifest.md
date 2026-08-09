# R6-T7 — v1 Message Struct Deletion Manifest

**Scope**: delete the v1 message structures (`Message`, `MessagePart`, `MessageMetadata`) from
`agena-runtime-contracts` and converge the re-export surface onto `part` / `provider_state`.
**Assumed landed**: T5 (provider projections → `&[Part]`) and T6 (session v1-bridge removal).
**This manifest is READ-ONLY analysis**: no file was modified; this document is the worklist for
executing T7.

---

## 1. v1 inventory — `crates/agena-runtime-contracts/src/message/`

The `message` module is a shim that mixes three things: the v1 structs to delete, the retained
`part` model (re-exported via `pub use crate::part::*`), and an alias to the relocated
`MessageProviderState`.

| Type | File | Verdict |
|---|---|---|
| `Message` | `src/message/message.rs` (whole file) | **DELETE** — v1. `prompt_text`, `prompt_parts`, `prompt_tool_result`, `as_text_lossy`, `visible_text_lossy`, `transition_state`, `push_part` all die with it. |
| `MessagePart` | `src/message/part/message_part.rs` (whole file) | **DELETE** — v1. Also the only holder of `activity_id` / `segment_id` / `operation_id` fields. |
| `MessageMetadata` | `src/message/metadata.rs` | **DELETE** — v1. File also carries `pub use crate::provider_state::MessageProviderState` (line 11) — that alias moves to the re-export surface, not into a deleted file. |
| `MessageProviderState` | `src/provider_state.rs` | **KEEP-SHARED** — T1 already moved it here; `message/metadata.rs:11` is a pure alias (verified: real definition is `provider_state.rs`). Survives at `crate::provider_state` and crate root. |
| `message/mod.rs` | `src/message/mod.rs` | **DELETE** (full convergence) — see §4. |
| `message/part/mod.rs` + `message/part/` | `src/message/part/` | **DELETE** — the entire directory; its only content is the shim + `message_part.rs`. |
| `part/*` (retained) | `src/part/*.rs` | **KEEP** — `PartContent`, `RuntimeActivity`, `Attachment*`, `OperationPart`, `OperationCompletion`, `HookPart`, `RequestPart`/`InteractiveRequestPart`, `NoticePart`, `SkillReference`/`SkillReferencePart`, tool-input structs. Re-exported by `message/mod.rs:10` `pub use crate::part::*`. |

**Not present** (grep-verified, so no inventory rows): `MessageRole`, `MessageContent`,
`MessageSegment`, `MessageStatus` do not exist in contracts `message`. Role is
`agena_domain::Role`, status is `agena_domain::ExecutionStatus`, content is the shared
`PartContent`. (The `MessageRole` / `MessageStatus` / `MessageMetadata` names that appear
elsewhere in the workspace are **agena-api's own resource types**, unrelated to contracts — see §3.)

Internal contracts hygiene (verified): no module outside `message/` references the v1 types. Only
stale doc comments at `src/part/mod.rs:3-4` ("`MessagePart` ... lives at [`crate::message::part`]")
must be edited.

---

## 2. Per-crate worklist

### 2a. `crates/agena-runtime-contracts` (the T7 deletion itself)

Delete files:
- `src/message/message.rs`
- `src/message/metadata.rs`
- `src/message/part/message_part.rs`
- `src/message/part/mod.rs`
- `src/message/mod.rs`

Edits:
- `src/part/mod.rs:3-4` — drop the "MessagePart lives at crate::message::part" comment.
- `src/lib.rs` — remove `pub mod message;` and `pub use message::*;`; add `pub use part::*;`
  (keeps the crate-root surface — `PartContent`, `NoticePart`, etc. currently reach the root via
  `message::*`); keep `pub mod part;` and `pub use provider_state::MessageProviderState;`.
- No other contracts change: `message` module gone entirely, `part` and `provider_state` are the
  canonical paths.

### 2b. Re-export points (5) — drop `message`, add `part` + `provider_state`

| File | Current | After |
|---|---|---|
| `crates/agena-runtime-session/src/lib.rs:26` | `pub use agena_runtime_contracts::{authorization, identity, message, permission};` | `pub use agena_runtime_contracts::{authorization, identity, part, permission, provider_state};` |
| `crates/agena-runtime-tools/src/lib.rs:11` | `pub use agena_runtime_contracts::{authorization, identity, message, permission};` | `pub use agena_runtime_contracts::{authorization, identity, part, permission, provider_state};` |
| `crates/agena-bundled-plugins/src/lib.rs:32` | `pub use agena_runtime_contracts::{authorization, message, permission};` | `pub use agena_runtime_contracts::{authorization, part, permission, provider_state};` |
| `crates/agena-runtime/src/lib.rs:37` | `pub use agena_runtime_contracts::message;` | `pub use agena_runtime_contracts::{part, provider_state};` |
| `crates/agena-runtime-contracts/src/lib.rs` | `pub mod message;` … `pub use message::*;` | `pub use part::*;` (see 2a) |

These are the only four crates that re-export the `message` module (grep-verified:
`grep -rn "use .*message"` finds exactly session, tools, bundled-plugins, agena-runtime, plus the
contracts `pub use message::*`). Session additionally aliases `crate::message` internally, and
`agena_runtime::session` re-exports session — all covered by the session edit.

### 2c. Import renames — `message::X` → `part::X` (part types) / `provider_state::MessageProviderState`

Symbols affected (all are part-model or relocated types, **not** v1): `PartContent`,
`RuntimeActivity`, `AttachmentItem`, `AttachmentKind`, `AttachmentSource`, `OperationPart`,
`OperationCompletion`, `NoticePart`, `HookPart`, `RequestPart`, `InteractiveRequestPart`,
`SkillReference`, `SkillReferencePart`, `AskUserToolInput`, `TaskAccess`, `TaskToolInput`,
`ApplyPatchToolInput`, `GlobToolInput`, `GrepToolInput`, `ReadToolInput`, `ShellCommandInput`,
`ShellMonitorInput`, `ShellMonitorPatternKind`, `ShellToolInput`, `CronCreateToolInput`,
`CronDeleteToolInput`, `CronHistoryToolInput`, `CronJobControlToolInput`, `CronListToolInput`,
`CronMisfirePolicyInput`, `CronRetryPolicyInput`, `CronUpdateToolInput`, `ScheduleWakeupToolInput`,
`EnterSnapshotToolInput`, `ExitSnapshotToolInput`, `InteractionNotifyToolInput`,
`LspDefinitionToolInput`, `LspDiagnosticsToolInput`, `LspHoverToolInput`, `LspReferencesToolInput`,
`ToolSearchToolInput`, `WebFetchToolInput`, `WebSearchToolInput`, `ModelVisibleOutput`,
`MessageProviderState` (→ `provider_state::`).

Per-file edit list (exact import lines; files already off v1 by T5/T6 are the only ones left to
rename — none of these reference v1):

**agena-bundled-plugins** (path `crate::message::` → `crate::part::`):
- `src/plugins/provided/cron.rs:8` — Cron* + ScheduleWakeupToolInput inputs
- `src/plugins/provided/fs.rs:6` — ApplyPatch/Glob/Grep/Read inputs
- `src/plugins/provided/interaction.rs:3` and `:91` — AskUserToolInput, InteractionNotifyToolInput
- `src/plugins/provided/lsp.rs:9` — Lsp* inputs
- `src/plugins/provided/mcp.rs:22` — AttachmentItem/Kind/Source
- `src/plugins/provided/shell.rs:3` — ShellCommand/Monitor/Tool inputs
- `src/plugins/provided/tasks.rs:5` and `:1006` — TaskAccess, TaskToolInput
- `src/plugins/provided/workflow.rs:7` — AskUserToolInput, TaskAccess, TaskToolInput

**agena-runtime-tools** (path `crate::message::` → `crate::part::`):
- `src/tool/cron.rs:8` — Cron* inputs
- `src/tool/file_attachment.rs:7` — AttachmentItem/Kind/Source
- `src/tool/lsp.rs:18` — Lsp* inputs
- `src/tool/payload.rs:7` — the large tool-input glob (ApplyPatch…WebSearchToolInput)
- `src/tool/router.rs:9` — ApplyPatchToolInput, ShellToolInput
- `src/tool/snapshot.rs:6`, `src/tool/tests.rs:10` — Enter/ExitSnapshotToolInput
- (tool input structs are re-exported by `part/tool.rs`; `ToolSessionContext` already imported
  from the crate root in tools lib.rs:10, unaffected.)

**agena-runtime-provider-adapters** (path `agena_runtime_contracts::message::` →
`agena_runtime_contracts::part::`):
- `src/provider/amazon_bedrock.rs:31` — AttachmentItem, AttachmentKind
- `src/provider/anthropic.rs:23` — AttachmentItem, AttachmentKind
- `src/provider/gemini.rs:26` — AttachmentItem
- `src/provider/openai.rs:35` — AttachmentItem, AttachmentKind, AttachmentSource
- `src/provider/openai/openai_response_builders.rs` — `mod tool_api_history_tests` only; T3 rewrites
  it off v1; the part-type portion (`OperationPart`, `PartContent`) renames to `part::`.

**agena-runtime-provider** (path `agena_runtime_contracts::message::` → `part::` /
`provider_state::`):
- `src/provider/registry/mod.rs:22` — AttachmentItem, AttachmentKind
- `src/provider/wire_message.rs:23` — after T5 drops `Message` from this import, the rest
  (Attachment*, OperationPart, PartContent, RuntimeActivity) → `part::`; `wire_message.rs:341`
  `message::MessageProviderState` → `provider_state::MessageProviderState`.

**agena-runtime-session** (paths `crate::message::` and `agena_runtime_contracts::message::` →
`part::`):
- `src/session/manager/runs.rs:12`, `:754` — SkillReference, SkillReferencePart
- `src/session/manager/replies/replies_execution.rs:16` — InteractiveRequestPart, RequestPart
- `src/session/mod.rs:23-24` — `crate::message::PartContent` → `crate::part::PartContent`
- `src/session/transcript.rs:146`, `:312-313` — AttachmentKind/PartContent/RuntimeActivity,
  AttachmentSource → `part::`
- plus any part-type lines left in `prompt_window.rs`, `store.rs`, `processor.rs`,
  `processor/parts.rs`, `manager/mod.rs`, `manager/history.rs`, `manager/tests.rs` that survive T6.

**agena-application** (path `agena_runtime::message::` → `agena_runtime::part::`):
- `src/application.rs:1034-1037`, `:1053-1057` — PartContent, RuntimeActivity, NoticePart.

**agena-runtime** — no import renames needed beyond lib.rs:37 (its `runtime/host_client/mod.rs:11`
uses AskUserToolInput/Enter/ExitSnapshot via a `message::` path → `part::`).

---

## 3. Straggler list — must land BEFORE T7 (cross-crate blockers)

These are the only remaining v1 consumers in the workspace. T7 cannot delete the structs until
they are gone. Grep-verified across all crates under `crates/` (api-server, cli, web, storage,
storage-sqlite bench `v2_store.rs`, tui-backend, tui-app, tui-session, tui-transcript, client,
session-core are all **clean** — zero v1 references).

### 3a. `agena-runtime-provider` — NON-TEST v1 (T5)
- `src/provider/wire_message.rs` (the projection layer):
  - `pub fn project_persisted(message: &Message) -> Vec<WirePart>` (line 89)
  - `pub fn project_completion_input(message: &Message) -> CompletionInputMessage` (line 261)
  - `pub fn validate_provider_native_tool_history(messages: &[Message])` (line 360)
  - `pub fn project_persisted_text_lossy(message: &Message) -> String` (line 391)
  - import at line 23 mixes v1 `Message` with part types.
- Public API re-exports of the v1-taking fns must change signature to `&[Part]`:
  - `src/lib.rs:23-25` (`project_completion_input`, `project_session_parts`,
    `project_session_text_lossy`)
  - `src/provider/mod.rs:34-37` (same, plus `WirePart as ProjectedSessionPart`).
- **Dependency note (surprise)**: `agena-runtime-provider` does **not** depend on `agena-storage`
  (verified `Cargo.toml`: only `agena-domain`, `agena-runtime-contracts`, …). The v2 `Part` type
  lives at `agena-storage::store::types::Part` (line 155). T5 must either add the `agena-storage`
  dep so the projection can name `Part`, or move the projection into session (which already has
  both deps). This is a T5 prerequisite, flagged so T7's "verify" stays honest.

### 3b. `agena-runtime-provider` + adapters — TEST v1 (T3, provider test decoupling)
- `wire_message.rs` tests (assistant_operation / Message::prompt_* at :928, :949, :993, :1026,
  :1141; import :923)
- `chat_wire.rs` tests (:72 import; `Message::prompt_parts` :397, :425)
- `multi_adapter.rs:990` — `Message::prompt_text`
- `registry/completion.rs` tests (:1565 import; `completed_help_message` :1753, :1788, :2219)
- adapters: `openai/openai_response_builders.rs` `mod tool_api_history_tests` (:882 import, many
  `Message::`/`MessagePart::` sites), `gemini/gemini_adapter.rs` tests (:871, :877, :939).

### 3c. `agena-runtime-session` — v1-bridge removal (T6)
Non-test v1 consumers (each file's v1 surface):
- `session/prompt_window.rs` — the largest consumer: `Vec<Message>` throughout,
  `Message::prompt_text` (:208, :227), `project_session_parts(&Message)` (:305, :424),
  `project_session_text_lossy` (:870, :919), plus `MessageMetadata` (:211, :237), `MessagePart`
  (:1079-1163).
- `session/store.rs` — the v1 bridge: `messages_from_parts` (:562), `message_from_run` (:599),
  `message_from_singleton` (:626), `metadata_from_parts` (:639), `serialize_part_content` (:693),
  `part_to_message_part` (:766), `part_content_from_value` (:885), and the typed→v1 rebuild
  helpers `part_content_from_typed` (:900), `operation_from_tool_call` (:977),
  `attachment_from_file_ref` (:1055), `skill_reference_from_skill_ref` (:1157),
  `user_problem_from_error` (:1190), `interaction_from_content` (:1258). **These v1-rebuild
  helpers become dead exactly when the bridge is removed (T6) and are deleted there.**
  `session/part_content.rs` (the typed layer being moved to contracts by T5) survives — it has no
  v1-rebuild code of its own; the rebuild direction lives in `store.rs`.
- `session/processor.rs` (:9 import), `processor/parts.rs` (MessagePart projection :53, :109,
  :276, :341, :436), `processor/run.rs` (:154 MessageMetadata), `processor/tool_calls.rs`
  (MessagePart :40, :229), `processor/helpers.rs` (MessageProviderState :60).
- `session/doom_loop.rs:9,14` — `detect(&[Message])`.
- `session/cost.rs:11,143` — `summarize(&[Message])` reading `.usage` / `.metadata`.
- `session/transcript.rs:145` — `from_message_lossy(&Message)`.
- `session/manager/history.rs:4` — Message/MessagePart; `manager/mod.rs:14,60` —
  `completion_request(Vec<Message>)` + `project_completion_input`; `manager/compact.rs:462`
  `project_completion_input` over historical v1 messages (+ `MessageMetadata` in test :886);
  `manager/replies.rs` / `replies_execution.rs` / `replies_state.rs`; `manager/tests.rs:32`.

### 3d. Non-contracts lookalikes — NOT v1, T7 leaves them alone
Grep hits for `Message*` that are **unrelated type families** and must not be "fixed" by T7:
- `agena-api/src/resource.rs` — its own `MessageRole`, `MessageStatus`, `MessageMetadata`,
  `MessageResource` wire types (no contracts import; verified).
- `agena-api/src/message_part.rs` — `MessagePartResource`, `MessagePartKindResource`,
  `MessagePartDetailResource` (no contracts import; verified).
- `agena-runtime-session/src/session_query_service.rs` — `SessionProjectedMessagePart` (session's
  own query type).
- `agena-tui-session/src/lib.rs:115` — `MessagePartChanged`; all tui crates consume agena-api
  resource types, not contracts message (verified: tui-app/tui-session/tui-transcript import no
  `agena_runtime_contracts::message`).
- `agena-storage-sqlite/benches/v2_store.rs` — pure v2, zero v1 (verified).

---

## 4. Converged re-export surface + identifier audit

### 4a. Post-T7 contracts surface
```
// crates/agena-runtime-contracts/src/lib.rs
pub mod authorization;
pub mod identity;
pub mod part;          // was message/part — now the only content path
pub mod permission;
pub mod provider_state; // MessageProviderState home (T1)
pub use part::*;        // preserves the crate-root surface formerly via message::*
pub use provider_state::MessageProviderState;
```
The `message` module is **gone**. There is no shim left. All four re-export crates (§2b) point at
`part` + `provider_state`. `agena_domain::Role` / `ExecutionStatus` / `PartKind` remain the
authoritative role/status/kind types (domain stays — see §4c).

**Risk-reduced alternative** (if a single mechanical pass over ~40 import sites is unwanted):
T7 could keep `src/message/mod.rs` as a two-line alias
(`pub use crate::part::*; pub use crate::provider_state::MessageProviderState;`) with zero external
churn, then a follow-up deletes the alias and applies §2c. Not recommended as the end-state: the
identifier audit (below) explicitly wants the `message` name gone, and a pure alias perpetuates two
paths to every part type.

### 4b. Identifier audit checklist
- [ ] `message` module name — eliminated by §2a/§2b (the biggest lingering v1 name; ~40 import
      sites + 5 re-export points).
- [ ] `Message` / `MessagePart` / `MessageMetadata` — gone from contracts.
- [ ] `segment_id`, `activity_id`, `operation_id` — die with contracts `MessagePart`. Mirror
      fields remain in **other** type families (out of T7 scope, flag for their owners):
      `agena-api/src/message_part.rs:36` (`segment_id`) and
      `session/session_query_service.rs:89` (`SessionProjectedMessagePart.segment_id`) — these are
      agena-api/session types, deleted/renamed by their own tracks, not T7.
- [ ] `runtime_activity` (snake) — zero hits workspace-wide already.
- [ ] `execution_status` — stays (domain-owned, see 4c); `part_state_from_execution_status` /
      `execution_status_from_part_state` in `session/store.rs` are storage-enum mappings that
      survive T6.

### 4c. `agena_domain` types stay in domain (verified, do not re-litigate)
- `ExecutionStatus` + `ExecutionStatusTransitionError` — `agena-domain/src/execution_status.rs`,
  exported at domain `lib.rs:128`.
- `PartKind` — `agena-domain/src/part_kind.rs`, exported at domain `lib.rs:162`.
- Dependency direction verified: `agena-runtime-contracts/Cargo.toml:10` has
  `agena-domain = { workspace = true }`; `agena-domain/Cargo.toml` has **no** contracts dep.
  Moving these to contracts would create a cycle. **They stay in domain.** They are not part of the
  contracts v1 inventory; their consumers (session, provider, tui, agena-api `message_part.rs`) use
  the `agena_domain` path and are untouched by T7.

---

## 5. Verify (cargo commands proving v1 is gone)

```bash
# 1. No v1 struct in contracts.
grep -rn "MessagePart\|MessageMetadata" crates/agena-runtime-contracts/src   # → nothing
grep -rn "pub struct Message\b\|struct Message\b" crates/agena-runtime-contracts/src  # → nothing

# 2. No module-path references to the deleted message module (whole workspace).
grep -rn "agena_runtime_contracts::message\|agena_runtime::message\b\|crate::message\|super::message" crates --include="*.rs"   # → nothing
# (agena-api `resource.rs`/`message_part.rs` and tui crates are agena-api types — excluded by construction.)

# 3. No v1 constructors remain.
grep -rn "Message::prompt_text\|Message::prompt_parts\|MessagePart::from_content" crates --include="*.rs"  # → nothing

# 4. No `message` module re-export remains.
grep -rn "use .*::message;\|use .*::message$\|use .*message;$\|pub use agena_runtime_contracts::message" crates --include="*.rs"  # → nothing

# 5. Workspace builds.
cargo check --workspace          # green
cargo test -p agena-runtime-contracts   # green (its own tests move/delete with the module)
cargo clippy --workspace -- -D warnings
```

After T7, `agena_runtime_contracts::part` and `agena_runtime_contracts::provider_state` are the
only content paths; `agena_runtime::part`, `agena_runtime_session::part`, `agena_runtime_tools::part`,
`agena_bundled_plugins::part` are the re-export surfaces.
