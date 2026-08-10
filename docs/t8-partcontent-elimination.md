# R6-T8 — v1 `PartContent` / `RuntimeActivity` Elimination Plan

**Goal**: after T7 (v1 `Message` deletion, branch `parallel/r6-t7-final`) and T6 (session
v1-bridge removal, worklist `docs/t6-v1-bridge-worklist.md`), delete the v1 leftover
`PartContent` enum so the v2 content model (`TypedContent`) is the *single* content type.
**No code may name `PartContent` or `RuntimeActivity` after this task.**

**Scope note**: this plan is written against the post-T7 / post-T6 tree. The current
`research/db-design-audit` worktree does **not** compile (mid-T6; e.g.
`prompt_window.rs:924` passes a v1 `Message` to the now parts-native
`project_session_text_lossy(&[Part])`). T8 stages must be executed after those two merges.

---

## 1. Verdict

**Full elimination is feasible with NO change to any external wire or serialized shape.**

- `PartContent` / `RuntimeActivity` are **never serialized across a process boundary**.
  Zero references in `agena-api`, `agena-api-server`, `agena-web`, `agena-client`,
  `agena-lsp`, `agena-mcp-server`, `agena-plugin-sdk`, or `agena-runtime/src`.
  - Provider path: storage `Part` → `decode_part_content` → `PartContent` → match →
    `WirePart` (no serde derive) → adapter payloads. `PartContent` is a transient
    in-process decode bridge only.
  - Notification path: `application.rs` copies `NoticePart` fields into a plain
    `agena_runtime_notifications::Notification` (`aggregate.rs::from_notice_part`); no
    `PartContent`/`NoticePart` is retained on the wire. JSON-RPC / REST / SSE surfaces
    use `agena-api` v2 resource types (`MessagePartResource`, `SessionTranscriptPart`,
    `MessagePartDetailResource`) which are independent of the v1 enums.
- The **DB `content` column** already stores canonical `TypedContent` JSON:
  `store.rs::part_content_to_value` produces exactly the typed `*Content::as_value()`
  payloads (with an empty `extra`). Post-T8 writes are `TypedContent::X(x).as_value()`
  — **byte-identical to today**. No schema / column change.
- The v1 payload structs (`OperationPart`, `AttachmentPart`, `SkillReferencePart`,
  `RequestPart`, `HookPart`, `ErrorPart`, `NoticePart`, `ReasoningPart`) **survive** —
  they ride losslessly in `TypedContent`'s `extra` bucket and are recovered by the
  existing `*_from_*` helpers. Only the two wrapper enums and the fold die.

**One shape that must stay byte-identical**: the canonical `(kind, content)` rows in
`parts.content`. Preserved because the write path is unchanged modulo dropping the
`PartContent` dispatch wrapper.

---

## 2. Target architecture

### 2.1 What dies

| Symbol | Location | Fate |
|---|---|---|
| `enum PartContent { Text, Activity }` + all constructor helpers (`text`, `reasoning_summary`, `attachments`, `request`, `operation`, `skill_reference`, `hook`, `notice`, `error`) and methods (`kind`, `text_value`, `reasoning_summary_value`, `append_*_delta`) | `crates/agena-runtime-contracts/src/part/content.rs` (whole file, `~31-148`) | **DELETE** (file has nothing else) |
| `enum RuntimeActivity` (8 variants) | same file `:14-23` | **DELETE**. Variants map 1:1 onto `TypedContent`: Reasoning→Think, Operation→ToolCall, Resource→FileRef, SkillReference→SkillRef, Interaction→Interaction, Hook→Hook, Error→Error, Notice→Notice. |
| `pub use content::{PartContent, RuntimeActivity}` + `mod content;` | `crates/agena-runtime-contracts/src/part/mod.rs:8,14` | **DELETE** |
| `pub fn decode_part_content(kind, value) -> Result<PartContent>` | `crates/agena-runtime-contracts/src/part_content.rs:537-540` | **DELETE** — exists only to return `PartContent` |
| `pub fn part_content_from_typed(TypedContent) -> PartContent` (the fold) | `part_content.rs:551-598` | **DELETE** — the 2-arm projection |
| `pub struct TextPart { text, synthetic }` | `crates/agena-domain/src/message_activity_values.rs:6-10` | **DELETE (optional, stage 5)** — after T7+T8 the only remaining references are the fold and `part/content.rs`; `TextContent` (typed) subsumes it. `ReasoningPart`/`ErrorPart` in the same file survive. |

### 2.2 What survives

- **`TypedContent`** + all per-kind `*Content` structs + **`decode(kind, value)`**
  (`part_content.rs:487-523`) — the single content model. **This is the keeper.**
- The `*_from_*` rich-view extractors in `part_content.rs` — currently private and only
  called from the fold; re-sign as `pub`/`pub(crate)` `&`-taking helpers:
  `operation_from_tool_call(&ToolCallContent) -> OperationPart` (`:603`),
  `attachment_from_file_ref(&FileRefContent) -> AttachmentPart` (`:627`),
  `attachment_source_from_file_ref` (`:671`), `skill_reference_from_skill_ref` (`:711`),
  `user_problem_from_error(&ErrorContent) -> UserProblem` (`:723`),
  `interaction_from_content(&InteractionContent) -> RequestPart` (`:756`).
- All v1 payload structs in `agena-domain` / `part/*` (OperationPart, AttachmentPart,
  SkillReferencePart, RequestPart/InteractiveRequestPart, HookPart, NoticePart,
  ErrorPart, ReasoningPart, tool-input structs, `tool_output_content_blocks`).
- `store.rs`'s inverse v1→typed serializers — `tool_call_from_operation` (`:890`),
  `file_ref_from_attachment`, `skill_ref_from_reference`, `error_from_problem`,
  `interaction_from_request` — they become the **direct** way to serialize a v1 payload
  struct to canonical JSON (today they are only reachable through the `part_content_to_value`
  dispatch).
- `agena-tui-transcript`'s `TranscriptPartContent` / `TranscriptActivityContent`
  (render_model.rs:73/81) — the TUI's own **presentation** classification, kept as-is.
  **The TUI crates never import contracts `PartContent`** (see §5).

### 2.3 The migration seam (how every consumer changes)

Every production consumer reaches `PartContent` through one of two chokepoints:

| Today | After T8 |
|---|---|
| `decode_part_content(&part.kind, &part.content)` → `PartContent` (store.rs `part_content_from_value` `:881`, wire_message.rs `projected_content` `:97`) | `decode(&part.kind, &part.content)` → `TypedContent` |
| `part_content_to_value(&PartContent::X(x))` (store.rs `:707`, ~15 sites) | `x_to_canonical(x).as_value()` — direct call of the existing typed serializer (e.g. `tool_call_from_operation(&op).as_value()`, `interaction_from_request(&req).as_value()`, `TextContent{...}.as_value()`) |
| `new_part_from_content(kind, role, &PartContent::X(x), state)` (store.rs `:807`, ~6 sites) | `new_part_from_content(kind, role, &TypedContent::X(x), state)` — kind derived from the variant, value via `as_value()` |
| `match PartContent { Text|Activity(RuntimeActivity::V(p)) => ... }` | `match TypedContent { Text|Think|ToolCall|... => ... }` — where rich v1 fields are needed, recover via the `*_from_*` helper (e.g. `TypedContent::ToolCall(tc) => { let op = operation_from_tool_call(&tc); ... }`) |
| `serde_json::from_value::<PartContent>(part.content)` (application.rs `:1034`) | `decode(&part.kind, &part.content)` → `TypedContent::Notice(nc)` |

---

## 3. Consumer census

Counts (production, excluding T7-dying `contracts/message/*` and T6-dying `processor/*`):
**~25 construct sites across ~12 files; ~20 match sites across ~9 files; ~10 carry
positions across ~7 files.**

### 3.1 `agena-runtime-contracts` (the definitions — die)

| File:line | Use | After |
|---|---|---|
| `part/content.rs:14-148` | `RuntimeActivity` + `PartContent` + helpers | DELETE whole file |
| `part/mod.rs:8,14` | re-exports | DELETE |
| `part_content.rs:537-540` | `decode_part_content` | DELETE |
| `part_content.rs:551-598` | `part_content_from_typed` fold | DELETE |
| `part_content.rs:603-784` | `*_from_*` helpers | KEEP, make `pub(crate)`/`pub`, re-sign to `&`; add `notice_part_from_notice_content` (extract the fold's Notice arm) |
| `message/message.rs:33,67,100-135` | `Message::prompt_text`/`prompt_tool_result`/`as_text_lossy` | dies with T7 |
| `message/part/message_part.rs:40,49,60,105-121,206-252` | `content: Option<PartContent>`, constructors, name/summary-from-content | dies with T7 |

### 3.2 `agena-runtime-provider` (1 file)

| File:line | Use | After |
|---|---|---|
| `provider/wire_message.rs:97-99` `projected_content` | CONSTRUCT via `decode_part_content` | return `Option<TypedContent>` via `decode().ok()` |
| `provider/wire_message.rs:124-211` `project_persisted` (re-exported `project_session_parts`) | MATCH `PartContent::Text/Activity` | match `TypedContent`; `ToolCall`→ToolCall/ToolResult, `Think`→Reasoning, `SkillRef`→SkillReference, `FileRef`→Attachment; skip Run/PasteRef/ToolResult/Compaction/Notice/Hook/Interaction/Error exactly as the fold degraded them today (preserves wire output) |
| `provider/wire_message.rs:468-496` `parts_as_text_lossy` | MATCH | same typed match (Text→text, Think→preferred_text, SkillRef→model_context_text, ToolCall→operation_text_lossy) |
| `provider/wire_message.rs:386-404` `validate_provider_native_tool_history` (test) | MATCH Operation | typed match |

### 3.3 `agena-application` (1 file)

| File:line | Use | After |
|---|---|---|
| `application.rs:1034-1041` `notification_from_session_change` | CONSTRUCT+MATCH — `serde_json::from_value::<PartContent>` then `RuntimeActivity::Notice` | `decode(&part.kind, &part.content)` then `TypedContent::Notice(nc)` → `from_notice_part(&notice_part_from_notice_content(nc), ...)`. **Also fixes a latent staleness bug** — today the column holds canonical `NoticeContent` JSON which does **not** decode as v1 `PartContent`, so notice banners are silently dropped (see §5, risk 1). |
| `application.rs:1049-1139` (tests) | CONSTRUCT `PartContent::text`/`Activity(Notice)` fixtures | build canonical `NoticeContent` JSON (or use `decode`) |

### 3.4 `agena-runtime-session` (the bulk)

**Serialization core — `session/store.rs`**

| Line | Use | After |
|---|---|---|
| `:686-700` `serialize_part_content` | CONSTRUCT/MATCH — injects `operation_id` into `PartContent::Activity(Operation)` metadata, then `part_content_to_value` | operates on `TypedContent`; inject into `extra["operation"]` (re-serialize `OperationPart` after mutating `.metadata`) |
| `:707-755` `part_content_to_value` | CONSTRUCT — the v1→typed dispatch | DELETE dispatch; keep the per-variant typed serializers, call `.as_value()` directly at each site |
| `:759-801` `part_to_message_part` | MATCH — status from `Operation`/`Interaction(UserInput)` | match `TypedContent::ToolCall` (recover `OperationPart` via `operation_from_tool_call`) / `TypedContent::Interaction` |
| `:807-826` `new_part_from_content(kind, role, &PartContent, state)` | CARRY/CONSTRUCT | re-sign to `&TypedContent`; kind from variant |
| `:828-857` `part_summary` | MATCH all variants | typed match (text/think/tool_call/error/skill_ref/interaction/hook/notice/file_ref) |
| `:881-884` `part_content_from_value` | CONSTRUCT via `decode_part_content` | return `Result<TypedContent>` via `decode` (or rename to `typed_content_from_value`) |
| `:1440-1553` (tests) | CONSTRUCT `PartContent::text`/`operation` | build typed content / canonical JSON |

**Execution-control plumbing (carry positions) — `session/mod.rs`, `manager/*`**

| File:line | Carrier | Replacement |
|---|---|---|
| `session/mod.rs:23-24` | `type ExecutionControl = agena_runtime::ExecutionControl<crate::message::PartContent>` (and `ExecutionRegistry`) | `...<TypedContent>` (the steer payload type) |
| `manager/mod.rs:87,107,111` | `SessionUserMessageRequest::new(..., parts: Vec<PartContent>)`; `UserInputPart { content: PartContent }`; `text_or_runtime` | `Vec<TypedContent>`; `{ content: TypedContent }`; drop `text_or_runtime` (rename) |
| `manager/mod.rs:264,1033,1078,1142,1150` | `steer_rx: mpsc::UnboundedReceiver<Vec<PartContent>>` | `Vec<TypedContent>` |
| `manager/runs.rs:180,413` | `steer_rx` params | `Vec<TypedContent>` |
| `manager/replies/replies_state.rs:523` | `drain_steer_input(steer_rx: &mut mpsc::UnboundedReceiver<Vec<PartContent>>)` | `Vec<TypedContent>` |
| `manager/history.rs:127-136` | `steer_input(session_id, parts: Vec<PartContent>)` | `Vec<TypedContent>` |

**Constructors / matches — `manager/*`, `prompt_window.rs`, `doom_loop.rs`**

| File:line | Use | After |
|---|---|---|
| `manager/mod.rs:752-840` `part_contents_from_composer_document` | CONSTRUCT `PartContent::text` / `attachments` / `Activity(SkillReference)` | `TypedContent::Text(TextContent{..})` / `TypedContent::FileRef(FileRefContent{..})` / `TypedContent::SkillRef(SkillRefContent{..})` (reuse the existing typed serializers) |
| `manager/runs.rs:113,209,216,594-596` | CONSTRUCT `PartContent::text` / `skill_reference` for steer + subtask spawn | `TypedContent::Text` / `SkillRef` |
| `manager/sessions.rs:189,197` | CONSTRUCT `&PartContent::text(...)` for session-start patch | `&TypedContent::Text` |
| `manager/replies.rs:198-205` `operation_from_part` | CONSTRUCT+MATCH via `part_content_from_value` | `decode(...)` → `TypedContent::ToolCall(tc)` → `operation_from_tool_call(&tc)` |
| `manager/replies.rs:200` | MATCH `RuntimeActivity::Operation` | typed match |
| `manager/replies/replies_execution.rs:811,857,2535-2536,2575` | CONSTRUCT `PartContent::text` / `hook` / `Activity(Interaction)` into `new_part_from_content`/`part_content_to_value` | typed content / `interaction_from_request(&req).as_value()` |
| `manager/replies/replies_execution.rs:1560-1573,1735-1748,2463,2512,2851,2929,2985,3104` | CONSTRUCT `part_content_to_value(&PartContent::operation(op))` → canonical tool_call | `tool_call_from_operation(op).as_value()` (existing helper) |
| `manager/replies/replies_execution.rs:2418,2454,2622-2638,2844,2907,2975` | MATCH `PartContent::Activity(Operation)` after decode | `TypedContent::ToolCall` |
| `manager/replies/tool_failure.rs:407`; `tool_non_execution.rs:50,93,137,188` | CONSTRUCT `part_content_to_value(&PartContent::operation(...))` | `tool_call_from_operation(...).as_value()` |
| `manager/history.rs:442-453` `part_failure` | MATCH Operation/Error | typed match (ToolCall via extractor, Error) |
| `manager/history.rs:576-720` `activity_payload_from_part` | MATCH all variants → `ActivityPayload` | typed match; recover v1 structs via extractors |
| `manager/history.rs:1158-1209` `project_part_detail` | MATCH all variants → `SessionProjectedPartDetail` | typed match |
| `prompt_window.rs:309-330` `message_has_visible_prompt_payload` | MATCH Hook/Notice/Interaction/Error | typed match (`TypedContent::Hook|Notice|Interaction|Error`) |
| `prompt_window.rs:930-1048` `assistant_tool_call_payload_chars` / `tool_result_extra_payload_chars` / `assistant_prompt_message_without_local_tool_results` / `extend_completed_tool_outputs` | MATCH `RuntimeActivity::Operation` | `TypedContent::ToolCall` + `operation_from_tool_call` |
| `prompt_window.rs:1063,1284,1332,1573-1578` (tests) | CONSTRUCT `PartContent::operation`/`text`/`hook` | typed content / direct canonical JSON |
| `session/doom_loop.rs:9-63` | MATCH `PartContent::Activity(Operation)` on `&[Message]` | **post-T7**: re-sign to parts-native (input becomes `&[Part]` or a decoded typed slice); match `TypedContent::ToolCall`. Caller `replies_execution.rs:325` feeds `messages_from_parts(...)` today → replace with parts (T6 §5/§6 already touches this site). |
| `session/transcript.rs:145-201` `from_message_lossy` (test) | MATCH via `crate::message::Message` | dies with T7 (test-only projection) |

**T6-dying (processor/*) — verify they are PartContent-free after T6, don't re-derive:**

| File:line | Use | T6 fate |
|---|---|---|
| `processor/parts.rs:23-273,341-459` | `&mut Message` accumulator + `PartContent::text/Activity(Reasoning)` + `assistant_message_from_run_parts` | T6 §5 deletes the accumulator + projection |
| `processor/run.rs:173-470` | `let mut assistant = Message {...}`, `PartContent::Activity(Operation)` match | T6 §5.3 |
| `processor/tool_calls.rs:13-352` | `&mut Message` + `PartContent::operation(...)` | T6 §5.3 |

**T7-dying (contracts/message/*)**: `message/message.rs`, `message/part/message_part.rs` —
all `PartContent` uses there die with T7.

**T6-dying session plumbing**: `messages_from_parts` (store.rs:555) and
`list_projected_messages` (sessions.rs:550) — T6 §6 / §7.1. After T6, the session never
rebuilds a v1 `Message`, which removes the biggest `Vec<PartContent>`-adjacent surface.

### 3.5 The TUI crates — OUT OF SCOPE (correcting the task premise)

The task brief assumed the TUI imports contracts `PartContent`. **It does not.** Verified
with word-boundary greps:
- `agena-tui-transcript` / `agena-tui-app` reference **zero** contracts
  `PartContent`/`RuntimeActivity` and do not depend on `agena_runtime_contracts` at all.
- The TUI's "PartContent" is its own render-model enum `TranscriptPartContent`
  (`agena-tui-transcript/src/render_model.rs:73`) with presentation variants
  (`TranscriptActivityContent`, `:81`). It already decodes the v2 wire shapes directly:
  `parts.rs:197` `part_content(&SessionTranscriptPart)` matches `part.kind` strings and
  hand-decodes `part.content: Value` into `TranscriptPartContent` — no `PartContent` in
  the path. `transcript_state.rs:3064/3075` uses `agena_tui_transcript::TranscriptPartContent`.
- `app_tests.rs:355-373` — the `api_message_part!` macro matches literal token patterns
  `PartContent::text(...)` / `PartContent::Reasoning(...)` / `PartContent::Text(...)` and
  expands to `TranscriptFixture::*` builders; no `PartContent` type is in scope. The macro
  arms are cosmetic and can stay (they don't name the type) or be renamed.
- **No TUI snapshot golden files embed the `PartContent` serde shape** (no `.snap` in
  transcript crates; fixtures build v2 canonical JSON).

Consequence: the "TUI is the largest consumer" loop is wrong. The largest consumer is
`agena-runtime-session`. The primary iteration loop is `cargo check -p agena-runtime-session`.

---

## 4. Staged implementation plan

Ordering is bottom-up: migrate every consumer off the bridge, then delete the bridge.
Precondition: T7 + T6 merged into `research/db-design-audit` and the tree compiles
(`cargo check --workspace` green at the baseline).

**Stage 1 — provider consumes `TypedContent` (independent crate).**
Files: `crates/agena-runtime-provider/src/provider/wire_message.rs`
(`projected_content`, `project_persisted`, `parts_as_text_lossy`,
`validate_provider_native_tool_history`).
Keep `decode_part_content`/`part_content_from_typed` alive (session still uses them).
Verify: `cargo check -p agena-runtime-provider` (deps: contracts + storage, not session).

**Stage 2 — application consumes `TypedContent`.**
Files: `crates/agena-application/src/application.rs`
(`notification_from_session_change` + its test module).
Verify: `cargo check -p agena-application` (pulls session transitively — needs the
post-T6/T7 tree; this is the first stage gated on the merges).

**Stage 3 — session consumes `TypedContent` (largest).** Ordered so the crate stays
green between sub-steps; verify with `cargo check -p agena-runtime-session` after each:
- 3a `store.rs`: `part_content_from_value`→`decode`; `new_part_from_content` re-sign to
  `&TypedContent`; add `typed_content_to_value(&TypedContent) -> Value` (variant `.as_value()`);
  rewrite `serialize_part_content` (operation_id injection into `extra["operation"]`),
  `part_summary`, `part_to_message_part`. Delete `part_content_to_value` last, after its
  call sites are gone.
- 3b steer/execution plumbing: `session/mod.rs:23-24` aliases → `TypedContent`;
  `UserInputPart.content`; `SessionUserMessageRequest::new`; `steer_rx` types in
  `manager/mod.rs`, `manager/runs.rs`, `replies_state.rs::drain_steer_input`;
  `manager/history.rs::steer_input`.
- 3c constructors: `manager/mod.rs::part_contents_from_composer_document`,
  `manager/runs.rs` (steer + subtask), `manager/sessions.rs` patch.
- 3d `manager/replies*.rs`: `operation_from_part`, the
  `part_content_to_value(&PartContent::operation(...))` → `tool_call_from_operation(...).as_value()`
  sites, the interaction write → `interaction_from_request(...).as_value()`,
  the decode+match sites.
- 3e `prompt_window.rs`: the 5 production matches + test fixtures.
- 3f `manager/history.rs`: `part_failure`, `activity_payload_from_part`,
  `project_part_detail` (recover v1 structs via extractors).
- 3g `doom_loop.rs`: re-sign to parts-native input; caller `replies_execution.rs:325`.
- 3h `grep -rn 'PartContent\|RuntimeActivity' crates/agena-runtime-session/` → only the
  `processor/*` sites T6 already deleted should be gone; expect zero.
Verify: `cargo check -p agena-runtime-session`.

**Stage 4 — delete the definitions.**
Files: `crates/agena-runtime-contracts/src/part/content.rs` (delete file),
`part/mod.rs` (drop `mod content;` + re-export), `part_content.rs` (delete
`decode_part_content` + `part_content_from_typed`; make `*_from_*` pub + `&`-signed; add
`notice_part_from_notice_content`; fix the `#[cfg(test)]` module — the fold tests at
`:786+` and any PartContent-referencing tests die or move). Because Stages 1-3 removed
all callers, this is a single mechanical commit.
Verify: `cargo check -p agena-runtime-contracts && cargo check --workspace`.

**Stage 5 — optional `TextPart` cleanup (verify-first).**
Files: `crates/agena-domain/src/message_activity_values.rs`, `agena-domain/src/lib.rs`.
Only do this if `grep -rn 'TextPart' crates/` shows zero references outside agena-domain.
Verify: `cargo check --workspace`.

**Stage 6 — final sweep + tests.**
```
grep -rn '\bPartContent\b\|\bRuntimeActivity\b' crates/   # expect: zero
cargo check --workspace
cargo test  -p agena-runtime-contracts -p agena-runtime-provider \
            -p agena-runtime-session -p agena-application -p agena-tui-transcript
```

**Verify loops**: primary `cargo check -p agena-runtime-session` (fastest crate that
contains the bulk). `cargo check -p agena-tui-app` is a valid whole-app smoke (it pulls
`agena-runtime` → session transitively) but is NOT the iteration loop — the TUI itself
has zero PartContent references.

---

## 5. Risks / surprises

1. **`application.rs:1034` decodes the content column as v1 `PartContent` — already
   stale.** The session writes canonical `NoticeContent` JSON
   (`{"kind":...,"summary":...,"detail":...}`), which `serde_json::from_value::<PartContent>`
   (expecting `{"type":"activity","activity_type":"notice","payload":{...}}`) cannot parse.
   Notice banners are therefore **silently dropped today**. T8's migration to
   `decode(&part.kind, &part.content)` is a correctness fix, not a wire change; the
   existing test (`:1049+`) builds the v1 shape and must be rewritten to canonical JSON.
2. **The fold's lossy arms preserve provider wire output.** `project_persisted` must keep
   degrading Run/PasteRef/ToolResult/Compaction exactly as `part_content_from_typed` did
   (degrade→skip) or the provider's `WirePart` stream changes. No external contract, but
   behavioral parity to avoid regressions in tests / UI.
3. **`operation_id` injection lives in the serialization path.** Today
   `serialize_part_content` mutates `OperationPart.metadata` under
   `OPERATION_ID_METADATA_KEY` before serializing. After T8 the `OperationPart` rides in
   `extra["operation"]`; the injection must mutate `extra["operation"].metadata` and
   re-serialize — mechanical, but easy to lose. `replies.rs::operation_id_from_part`
   (`:210-218`) reads it back and must keep working.
4. **The rich v1 structs are only reachable through `extra`.** Any session code that reads
   fields not on the canonical `*Content` struct (e.g. `OperationPart.{summary,title,error,
   authorization,details,invocation}`, `AttachmentPart`, `SkillReferencePart.skills`) must
   route through the extractor helpers. The census above flags every such site; do not
   "simplify" by dropping the extra round-trip.
5. **`doom_loop.rs` takes `&[Message]`** — its input type dies with T7; the caller
   (`replies_execution.rs:325`) currently feeds `messages_from_parts(...)`, which dies
   with T6 §6. Plan T8's doom_loop re-sign in lockstep with T6 §5/§6 (parts-native input).
6. **`ExecutionControl<T>` / `ExecutionRegistry<T>` are generic over the steer payload.**
   The `PartContent` instantiation is a session-private alias (`session/mod.rs:23-24`,
   `pub(crate)`); no external crate instantiates it. Changing to `TypedContent` is
   contained — verify no other `ExecutionControl<...>` instantiation exists (checked: none).
7. **`text_value()` / `reasoning_summary_value()` / `append_*_delta` helpers die.**
   Their callers (`runs.rs:190,208`) switch to `TypedContent::Text(t) => Some(t.text)`.
   The streaming `append_*_delta` mutators live in the processor accumulator that T6
   deletes — after T6 no production code mutates rich content in place except the
   operation_id injection (risk 3).
8. **`part_content_from_typed`'s `Text` arm sets `synthetic` from the typed struct** —
   the typed round-trip is lossless; no behavior change expected. Watch the `synthetic`
   flag on steer/user text parts (defaults `false`).
9. **Serde shape note (informational):** `PartContent` is *internally* tagged
   (`tag="type"`, no `content`), `RuntimeActivity` is adjacently tagged
   (`tag="activity_type", content="payload"`). Neither shape is an external contract and
   neither appears in TUI fixtures or golden files — only the stale decode in risk 1
   ever assumed it.

---

## 6. Definition of done

- `grep -rn 'PartContent\|RuntimeActivity' crates/ --include='*.rs'` → zero (tests included).
- `cargo check --workspace` and the Stage-6 test set green.
- The only surviving content model in contracts is `TypedContent` + `decode(kind, value)`.
- DB `parts.content` rows unchanged (byte-identical writes via `*Content::as_value()`).
- This document stays uncommitted for the acceptance batch.
