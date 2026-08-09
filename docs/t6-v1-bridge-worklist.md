# R6-T6 Worklist: Remove the v1 message bridge from `agena-runtime-session`

Branch: `research/db-design-audit` (worktree `.agena/worktrees/db-design-audit`).
Generated 2026-08-10 as a read-only audit. Every `file:line` was verified against the
working tree at generation time. `crates/agena-tui-app/` is a parallel agent's
uncommitted work — **out of scope, never touch it.**

The v1 `Message` type (`crate::message::Message`, re-exported from
`agena-runtime-contracts` at `crates/agena-runtime-session/src/lib.rs:26`) is the
"legacy prompt/UI" projection that several internal surfaces still rebuild from the
v2 parts projection. This worklist removes it from `agena-runtime-session`.

---

## 0. State snapshot (read this first — several plan assumptions are already stale)

1. **`build_message` / `reserve_message_ids` do not exist anywhere in the repo** (not
   in any crate, not in any file, not in git on this branch). They were deleted in
   `a1430cc3` (P5) and `c7a3c96b` (A6). **Plan step 4 is already complete — nothing
   to do.** The only related survivor is `ProcessorPartIdAllocator::reserve`
   (`store.rs:495-498`), which is the *parts-native* negative-placeholder id path and
   is NOT the v1 allocator; it stays.
2. **`Session` has no `.messages` field** (`agena-runtime-session-core/src/model.rs:707-736`,
   parts-projection cache since A5'). Plan step 7 is therefore *not* "delete a field";
   it is deleting the now-dead `Vec<Message>` plumbing that still threads through the
   prompt/compaction/query surfaces after steps 1-6.
3. **The single biggest v1 construction site is the streaming `assistant: Message`
   accumulator inside the processor** (`processor/run.rs:173-182`, mutated through
   `processor/parts.rs` and `processor/tool_calls.rs`). This is *not*
   `assistant_message_from_run_parts`; it is a separate, larger inline `Message`
   that must be replaced by a parts-only accumulator. Plan step 5 does not call this
   out explicitly — it is the bulk of T6.
4. **`list_projected_messages` is the v1 bridge's escape hatch to other crates.**
   It is `messages_from_parts`-backed and consumed by `agena-application` and
   `agena-cli` (see §7). T6 cannot delete `messages_from_parts` without resolving
   these.
5. **The `project_*` functions live in `agena-runtime-provider`, not
   `agena-runtime-session`** (`project_persisted`, `project_completion_input`,
   `project_persisted_text_lossy` in `provider/wire_message.rs:89,261,391`, aliased to
   `project_session_parts` / `project_session_text_lossy` at `lib.rs:22-24`). The
   provider crate does **not** depend on `agena-storage` (which defines `Part`), and
   every provider-internal call site of `project_completion_input` is **test-only**
   (`chat_wire.rs:411,438`, `multi_adapter.rs:989`, `registry/completion.rs:1788,1792,2218`,
   gemini `gemini_adapter.rs:897,926,950`, openai `openai_response_builders.rs:920,921,975,1033,1167,1179,1183`).
   So T6 must not naively change these provider signatures (that would pull `agena-storage`
   into the provider crate and rewrite its test fixtures). Instead see §1.5 for the two
   viable options.
6. **`Session::active_window_parts()` has already landed** at
   `agena-runtime-session-core/src/model.rs:868-877` (R4b's API half), but
   **`prompt_window` is NOT yet switched to it** (zero call sites). R4b's
   prompt_window half is in flight. T6 must build on R4b — see §1.6.
7. `doom_loop::detect` (`doom_loop.rs:14`) and `cost::summarize` (`cost.rs:143`) are
   v1 `&[Message]` consumers — real T6 work, in scope (steps 2-3).
8. `MessageProviderState`-on-Message residues: `store.rs:604-609` (decode into
   `MessageProviderState` in `message_from_run`), `processor/parts.rs:372`
   (`assistant_message_from_run_parts` param), `processor/run.rs:618-620`,
   `processor/helpers.rs:58-60`. All die with the bridge.

---

## 1. Step 1 — `prompt_window` (v1 → parts)

`crates/agena-runtime-session/src/session/prompt_window.rs` (1594 lines) is the
largest bridge consumer. The plan's intent: the whole file stops materializing
`Vec<Message>` and instead operates on the parts window (`Session::active_window_parts()`).

### 1.1 The bridge root
- `prompt_window.rs:22` — `use super::store::messages_from_parts;` → delete (step 6 removes the fn).
- `prompt_window.rs:130-138` — `fn projected_messages(session) -> Vec<Message>` calls
  `messages_from_parts(session.parts())`. → Delete; callers get `session.active_window_parts()` directly.

### 1.2 Window selection (builds on R4b — do not redo)
- `prompt_window.rs:140-142` — `active_prompt_messages(session)` → becomes
  `active_window_parts`-based (already returns the R4b window). Signature becomes
  `&[Part]`-based.
- `prompt_window.rs:144-202` — `active_prompt_messages_for_model`:
  - :161 `return projected_messages(session)` → `session.active_window_parts()`.
  - :176-181 `projected_messages(...).filter(|m| m.id > compaction.compacted_through_message_id)`
    → the **v1 `compacted_through_message_id` filter is redundant** once R4b lands
    (window = parts after last compaction marker). Delete the filter, keep the window.
  - :193-196 same filter → delete.
  - :200 `projected_messages(session)` → `session.active_window_parts()`.
  - This is where R4b's prompt_window switch lands; T6 only finishes it.

### 1.3 Checkpoint/compaction synthetic messages (delete with machinery)
- `prompt_window.rs:204-224` — `checkpoint_recent_message(session, &PromptCompactionMessage) -> Message`.
- `prompt_window.rs:226-247` — `compaction_summary_message(session, summary) -> Message`
  (builds a synthetic user `Message::prompt_text`).
- `prompt_window.rs:1056-1077` — `synthetic_tool_result_message(...) -> Message`
  (`Message::prompt_parts` with an `OperationPart`).
- `prompt_window.rs:1001-1043` — `extend_completed_tool_outputs(&mut Vec<Message>, &Message, ...)`.
- `prompt_window.rs:970-999` — `assistant_prompt_message_without_local_tool_results(&Message) -> Option<Message>` (clones+mutates v1 Message).
- These exist to (re)shape the provider-visible transcript. Under parts they are
  replaced by parts-level transforms (or by `project_persisted`-style projection over
  `&[Part]`). `checkpoint_recent_message`/`compaction_summary_message` are only reachable
  from `active_prompt_messages_for_model`'s TextSummary branch; once compaction runtime
  is parts-native (§3, compact.rs) they become unreachable and are deleted.

### 1.4 Normalization / digest / continuation / budget (whole-file `&[Message]` → `&[Part]`)
All of the following currently take/return `Vec<Message>`/`&[Message]`; all switch to a
parts-native transcript. Exact anchors:
- `:278-298` `normalize_prompt_messages`
- `:300-302` `prompt_messages_for_request`
- `:304-325` `message_has_visible_prompt_payload` (uses `project_session_parts(message)` at :305)
- `:339-344` `approximate_prompt_payload_chars`
- `:364-401` `estimate_prompt_tokens_from_runtime` (reads `message.id` of the anchor at :384)
- `:403-419` `prompt_transcript_digest` / `prompt_prefix_transcript_digest`
- `:421-562` `messages_to_provider_transcript` (uses `project_session_parts(message)` at :424,
  `message.role`, `message.as_text_lossy()` at :477,:532)
- `:746-828` `evaluate_prompt_continuation` (finds anchor by `message.id` at :796-798,
  digest prefix at :807)
- `:606-630` `approximate_total_request_tokens` / `..._with_compaction`
- `:661-723` `build_prepared_prompt` (feeds `PreparedPrompt.messages: Vec<Message>` at :42)
- `:867-909` `project_transcript(&[Message], budget) -> String` (maps `project_session_text_lossy` at :870)
- `:918-923` `approximate_message_payload_chars` (calls `project_session_text_lossy` at :919)
- `:925-968` `assistant_tool_call_payload_chars` / `tool_result_extra_payload_chars` (iterate `message.parts`)
- `:1139-1167` `tool_execution_call_id` / `tool_result_output_text` (take `&MessagePart`)

### 1.5 The `project_*` projection functions (cross-crate; two options)
`prompt_window.rs:305,424,870,919` and `manager/mod.rs:60`, `manager/compact.rs:462`
are the runtime-session call sites of the provider-crate projectors. Switching them to
`&[Part]` has two viable paths — pick one and keep it consistent:

- **Option A (recommended): project inside runtime-session.** Add a runtime-session-local
  parts→`WirePart`/`CompletionInputMessage`/text projection (it already has
  `part_content_from_value`, `part_role_from_role`, `OPERATION_ID_METADATA_KEY`,
  `part_content_to_value` in `store.rs`). Change the runtime-session call sites to it and
  **stop importing** the provider `project_session_parts`/`project_session_text_lossy`
  aliases. The provider `project_persisted`/`project_persisted_text_lossy`/`project_completion_input`
  then have **zero production callers** in runtime-session; leave the provider functions in
  place (still used by provider test fixtures) or delete them together with their tests —
  they are not load-bearing for T6.
- **Option B: change the provider signatures to `&[Part]`.** This requires adding
  `agena-storage` to `crates/agena-runtime-provider/Cargo.toml` (currently not a dep)
  and rewriting every provider/adapters test call site listed in §0.5 to construct `Part`s.
  Higher blast radius; only worth it if the provider projectors must become the canonical
  parts projection.

Note: `project_operation_output` (aliased `project_session_tool_result_output`,
`provider/mod.rs:35`) already takes `(status, &OperationPart)` — parts-friendly; its
call sites at `prompt_window.rs:1085,1096,1122,1163` need only the `MessagePart`→`Part`
swap.

### 1.6 R4b coordination
`Session::active_window_parts()` exists (`agena-runtime-session-core/src/model.rs:868`).
R4b is switching `prompt_window`'s window selection onto it; T6 must **not redo** that.
By the time T6 runs, the v1 `compacted_through_message_id` filter at
`prompt_window.rs:179,195` should already be redundant; T6's prompt_window step is then:
delete the leftover in-memory `PromptCompactionMessage.compacted_through_message_id`
machinery if unreachable (see §4.1 for its construction sites) and finish the
`Vec<Message>`→`&[Part]` conversion of §1.4.

---

## 2. Step 2 — `doom_loop` (v1 → parts)

- `doom_loop.rs:14` — `pub fn detect(messages: &[Message], policy) -> Option<DoomLoopHit>`.
  Iterates `message.parts` in reverse looking for identical `Operation` invocations.
  → Rewrite to `pub fn detect(parts: &[Part], policy)` walking the parts projection
  directly (only assistant-role `Operation` parts participate; a non-tool part breaks
  the run just like the current `if latest_signature.is_some() { break }` at :31-34).
- `doom_loop.rs:9` — drop the `Message` import.
- Only production caller: `replies_execution.rs:325-327` —
  `crate::session::doom_loop::detect(messages_from_parts(session.parts())?.as_slice(), ...)`
  → `detect(session.active_window_parts(), ...)`.

---

## 3. Step 3 — cost / usage accounting (v1 → parts)

### 3.1 `cost.rs`
- `cost.rs:143` — `pub(crate) fn summarize(messages: &[Message]) -> SessionCostSummary`.
  Reads `message.role` (:147), `message.usage` (:150), `message.metadata.model_provider_id`/`model_id`
  (:154-155). → Rewrite to `summarize(parts: &[Part])`: usage/model identity must come from
  where the run now persists them. **Design 17.5 note:** usage is *not* on a part — it is
  persisted through the runtime anchor (`replies_execution.rs:1171-1181`,
  `SessionRunResult.usage` at `processor.rs:86`). Decide the durable home for cost input:
  either (a) fold usage from `session.runtime` prompt-token/usage records (already
  persisted), or (b) accept `&[Part]` + the usage list. Do not silently drop cost data.
- `cost.rs:150` — `message.usage` is only populated on v1 Messages via the bridge; in
  parts world it comes from the run-marker/usage path.

### 3.2 Callers / usage path
- `history.rs:970-980` — `session_cost_summary`: `messages_from_parts(session.parts())?`
  (:977) then `cost::summarize(&messages)` (:979). → parts-native.
- `history.rs:959-968` — `session_usage`: delegates to `SessionManager::session_usage`
  (`sessions.rs:310`), which is built on `active_prompt_messages`/`..._for_model`
  (`sessions.rs:323-334`), `estimate_prompt_tokens_from_runtime` (:417),
  `approximate_total_request_tokens_with_compaction` (:435). → these become parts-native
  with step 1.
- `replies_execution.rs:1171-1181` — `assistant_message.usage` → `record_prompt_tokens(...)`.
  `assistant_message` is the step-5 v1 projection; replace with `result.usage`
  (`SessionRunResult.usage`) + `result.assistant_message_id`.
- `replies_execution.rs:1189` — `assistant_message.id` → `result.assistant_message_id`.
- `processor/run.rs:617` — `assistant.usage = usage.clone()` (v1 accumulator) → moves into
  `SessionRunResult.usage` (already returned at :668).

---

## 4. Step 4 — delete `build_message` / `reserve_message_ids`

**Already done.** Zero references anywhere (verified repo-wide including git history:
removed by `a1430cc3` P5 / `c7a3c96b` A6 / `ac8b1512` P6). No work. Do not go hunting
for them. (The `ProcessorPartIdAllocator::reserve` placeholder path at
`store.rs:493-498`, `processor/tool_calls.rs:17,212` stays — it is the parts-native id source.)

### 4.1 `compacted_through_message_id` leftover machinery
- `agena-runtime-session-core/src/model.rs:151-156` — `PromptCompactionMessage { id, role, source, text }`
  (the "recent messages" snapshot type — v1-flavored).
- `model.rs:178-187` — `PromptCompactionRuntime.compacted_through_message_id: i64` (:180).
- Construction: `manager/compact.rs:569` (`compacted_through_message_id: boundary`),
  `compact.rs:576` (`crate::session::PromptCompactionMessage { ... }`), `compact.rs:837` (test),
  `compact.rs:615` (read of `runtime.compacted_through_message_id`), `replies_execution.rs:539`
  (read for auto-compaction boundary check).
- Once R4b makes the filter in prompt_window redundant and step 6 removes the v1 window,
  delete `PromptCompactionMessage` and `PromptCompactionRuntime.compacted_through_message_id`
  **if and only if** unreachable; the auto-compaction "already compacted at boundary" check at
  `replies_execution.rs:530-540` must be re-expressed against the last compaction marker
  part (which `active_window_parts` already encodes) instead of `compacted_through_message_id`.

---

## 5. Step 5 — delete `assistant_message_from_run_parts` (+ the streaming accumulator)

### 5.1 The function itself
- `processor/parts.rs:367-392` — `pub(crate) fn assistant_message_from_run_parts(...) -> Result<Message, AppError>`.
  → Delete (its logic is the v1 projection; the run loop no longer needs a v1 image).
- `processor.rs:115` — `pub(crate) use self::parts::assistant_message_from_run_parts;` → delete.
- `processor.rs:59` — doc-comment reference → update.
- Its private helpers die with it: `processor/parts.rs:401-432` `metadata_from_run_marker`,
  `:436-...` `project_run_part`. (`new_part_for_deferred_tool_part` at `:341-359` is
  parts-write machinery and survives, but change its `&MessagePart` param to the parts form.)

### 5.2 The only production caller
- `replies_execution.rs:1117-1127` — rebuilds `assistant_message` from
  `result.{assistant_message_id, message_state, run_marker, parts, provider_metadata, usage}`.
  → Delete the projection. Downstream uses of `assistant_message`:
  - :1128-1139 `prompt_transcript_digest(transcript_messages)` where `transcript_messages`
    = active window messages + `assistant_message.clone()` → with step 1 parts-native,
    push `result.parts` (or the marker+parts) onto the parts window and digest that.
  - :1171-1181 `assistant_message.usage` / `.id` → `result.usage` / `result.assistant_message_id` (§3.2).
  - :1189 `assistant_message.id` → `result.assistant_message_id`.
  - `SessionRunResult.message_state` (`processor.rs:74`) was kept solely to feed this
    projection → delete the field and its write at `run.rs:663` (`message_state: ExecutionStatus::Completed`)
    and `run.rs:595` (`message_state: terminal_status`).

### 5.3 The streaming `assistant: Message` accumulator (the real bulk)
`processor/run.rs` maintains a full v1 `Message` in parallel with `parts: Vec<Part>`:
- `run.rs:154-168` — `assistant_metadata = MessageMetadata { ... }` → build the run-marker
  content (already done via `run_marker_content` at the `start_run` call sites) instead.
- `run.rs:173-182` — `let mut assistant = Message { ... }` → delete; keep only `parts: Vec<Part>`.
- All downstream `&mut assistant` mutations (`complete_part_status`, `append_text_delta`,
  `persist_part_state`, `start_text_part`, `start_reasoning_part`, tool-call creators)
  must be rewritten to mutate the `Vec<Part>` accumulator (decode `Part.content` via
  `part_content_from_value` where they need rich `PartContent`; re-serialize via
  `part_content_to_value` on write). Exact anchors in `processor/run.rs`:
  `:227,254,259,286,291,328,333,362,367,407,453,455,479,484,502` (helper calls), `:617,620`.

#### Helpers that take `&mut Message` (rewrite to `&mut Vec<Part>` or delete)
- `processor/helpers.rs:7` `complete_part_status`, `:24` `cancel_nonterminal_parts`,
  `:28` `fail_nonterminal_parts`, `:32-...` `terminalize_nonterminal_parts`.
- `processor/parts.rs:20` `start_text_part` (also `MessagePart::from_content` at :53-59,
  `assistant.push_part` :60, `assistant.transition_state` :61-65),
  `:72` `start_reasoning_part` (:109-125), `:134` `append_text_delta` (reads `assistant.parts`
  :142-150), `:176` `append_reasoning_delta`, `:220` `persist_part_state`
  (uses `serialize_part_content(part)` at :236), `:270` `persist_deferred_tool_parts`
  (filters `assistant.parts` for `id < 0` placeholders :276-301).
- `processor/tool_calls.rs:10` `ensure_pending_tool_call_part`, `:117` `finalize_pending_tool_calls`,
  `:188` `ensure_provider_native_tool_call_part`, `:287` `complete_provider_native_tool_call_part`
  (all take `&mut Message`; `transition_state` at :51,:241).
- `store.rs:693` `serialize_part_content(part: &MessagePart)` — used only by
  `parts.rs:198,236` (and a test at :1776). Either delete (inline `operation_id`-metadata
  injection into the parts-native write) or re-sign to take the decoded `PartContent` +
  `operation_id` directly. `store.rs:714` `part_content_to_value(&PartContent)` survives
  (widely used, parts-native).

### 5.4 Tests that exercise the projection (migrate, don't delete)
- `processor.rs:227-293` `run_parts_project_onto_the_legacy_message` (uses the fn at :261).
- `replies_execution.rs` doom-loop and turn-continuation tests that assert on v1 messages.
- `processor/helpers` behavior is covered indirectly; re-anchor to parts.

---

## 6. Step 6 — delete `messages_from_parts` / `message_from_run` / `message_from_singleton`

### 6.1 The bridge in `store.rs`
- `store.rs:562-597` — `pub(crate) fn messages_from_parts(parts: &[Part]) -> Result<Vec<Message>, AppError>`.
- `store.rs:599-624` — `fn message_from_run(...)`.
- `store.rs:626-637` — `fn message_from_singleton(...)`.
- `store.rs:639-674` — `fn metadata_from_parts(...)` (only called by `message_from_run`).
- `store.rs:766-...` — `fn part_to_message_part(...)` (only called by the two above).
- `store.rs:524-532` — `session_from_view` doc comment "until R6 removes it" → update.
- Delete all of the above together. `role_from_part_role` (:1288),
  `execution_status_from_part_state` (:1307) etc. survive (used by processor/manager).

### 6.2 Production call sites to re-express parts-natively (each with its replacement)
- `prompt_window.rs:131` (`projected_messages`) → `session.active_window_parts()` (§1.1).
- `manager/permission_service.rs:277` — `messages_from_parts(session.parts()).unwrap_or_default()`
  then `project_transcript(&messages, ...)` (:287) and `messages.len()` as cache key (:284,:289)
  → project over `session.active_window_parts()`; message-count key becomes part-count or a
  parts digest.
- `manager/sessions.rs:550-562` — `list_projected_messages` (see §7).
- `manager/sessions.rs:567-582` — `has_user_message_idempotency_key`: iterates messages for
  `metadata.idempotency_key` → scan user-role run markers' content (`run_marker_content`
  carries it) or the persisted marker content directly.
- `manager/runs.rs:118-154` — `read_subtask_output`: `messages_from_parts(child.parts())?`
  (:129) then `message.id`, `visible_text_lossy()`, `role`, `created_at` → walk parts
  directly (role from `Part.role`, text via `part_content_from_value`, id = `part_id`,
  created_at from `created_at_ms`).
- `manager/history.rs`:
  - `:27` `fork_session` — resolves `at_message_id` (a run-marker part id) to the message's
    inclusive cutoff via `messages.iter().find(|m| m.id == part_id)` + `message_inclusive_cutoff`.
    → replace with `last_part_id_for_run_marker(parts, marker_part_id)` (the run marker is the
    message id; the cutoff is the last `Part` with `run_id == marker_part_id`).
    `message_inclusive_cutoff(&Message)` at `history.rs:218-224` → delete.
  - `:150` `rewind_session` — via `user_message_id_for_turn` (`:234-259`, scans messages for
    `metadata.conversation_turn_id` then nearest prior `Role::User`). → scan assistant run
    markers for the UUID pair (as `metadata_from_parts` did) and then the preceding user marker.
  - `:272,322,354,384` — `transcript_snapshot_from_session` / `assistant_reply_snapshot` /
    `canonical_turn_span` / `assistant_reply_fields`: the whole v1-Message transcript-snapshot
    derivation (`history.rs:266-526`, `content_document_from_message` :459,
    `transcript_node_from_part` :477, `activity_payload_from_part` :576, plus helpers through
    ~:760). → re-implement directly over `&[Part]` (role grouping by run markers; the same
    content nodes derive from decoded part content). This is the largest history.rs chunk.
  - `:802` `session_presentation` — `message_count = messages_from_parts(...).len()` → count
    run-marker parts (`session.parts().iter().filter(|p| p.is_run_marker()).count()`).
  - `:977` `session_cost_summary` → §3.
- `manager/replies/replies_execution.rs` (9 production call sites):
  - `:261-265` last user message id (observed user id) → last user-role run-marker part id.
  - `:277-288` latest user message → last user-role run-marker part (id + `model_turn_id`
    recovered from marker content).
  - `:326` doom loop → §2.
  - `:402-412` last assistant text (`m.as_text_lossy()`, source filter) → last
    assistant-role run-marker's decoded text; `source` = run-marker content `source` field.
  - `:530-533` last message id (auto-compaction boundary) → last run-marker part id; §4.1.
  - `:575` `message_count` (PreRunInput) → run-marker count.
  - `:632-636` `active_model_turn_id` recovery → from assistant marker content.
  - `:643` `message_count` (PostRunInput) → run-marker count.
  - `:1067-1070` `completion_parent_message_id` → last run-marker part id.
  - `:1117` assistant projection → §5.2.

### 6.3 Test call sites (migrate to parts or to the new parts-native helpers)
- `store.rs:1714`, `manager/tests.rs:345,381,415,425,585`,
  `prompt_window.rs:1363,1389,1456,1470,1495,1507` (some already construct `Part` fixtures).

---

## 7. Step 7 — clear `.messages` residues

`Session` itself has no `messages` field (§0.2). The residues are local `Vec<Message>`
plumbing that dies with steps 1-6:
- `manager/mod.rs:46-75` — `fn completion_request(... messages: Vec<Message> ...)`; the
  `.map(crate::provider::project_completion_input)` at `:60` is the last v1 projection in
  the run path. Its two callers feed it `Vec<Message>`: `replies_execution.rs:1018-1026`
  (`prepared.messages`) and `compact.rs:383-391` (`inputs.messages`). → Change the
  parameter to parts (or to `&[CompletionInputMessage]` pre-projected by the step-1
  projection) and drop `project_completion_input`.
- `manager/compact.rs:37` — `struct PromptInputs { messages: Vec<Message>, ... }` →
  parts window. `compact.rs:208,386` read `.messages`.
- `prompt_window.rs:42` — `PreparedPrompt.messages: Vec<Message>` → parts (or
  `Vec<CompletionInputMessage>`); `replies_execution.rs:1007,1021` read it.
- `completion_request.rs:25` (`CompletionRequestInputs.messages`) is already
  `Vec<CompletionInputMessage>` (provider contract, parts-native) — unchanged.

### 7.1 `list_projected_messages` — the cross-crate v1 residue (must resolve before T6 closes)
- `sessions.rs:550-562` — `SessionManager::list_projected_messages` returns `Vec<Message>`
  via `messages_from_parts`.
- `session_query_service.rs:369` — trait method `list_projected_messages`;
  `history.rs:893-926` — impl builds `agena_runtime::SessionProjectedMessage` per v1 Message
  (maps `message.{id,role,state,created_at,metadata,usage,parts}`).
- **External production consumers (do NOT break):**
  - `agena-application/src/service/execution.rs:501-510` — `session_transcript_parts` →
    `project_session_transcript(&messages)` at `agena-application/src/session.rs:54-...`
    (renders `SessionProjectedMessage`).
  - `agena-cli/src/cli/cli_render.rs:673` (`render_debug`) and `:851` (`last_assistant_text_from_projection`).
- **Resolution (pick one, both are outside the 7 numbered steps):**
  - (a) Re-implement `list_projected_messages` parts-natively: build
    `SessionProjectedMessage`/`SessionProjectedMessagePart` directly from `&[Part]`
    (decode via `part_content_from_value`), preserving the wire shape so
    `agena-application`/`agena-cli` compile unchanged.
  - (b) Replace the whole `SessionProjectedMessage` read contract with a parts-native
    transcript contract and migrate `agena-application::project_session_transcript` +
    the two `agena-cli` call sites (larger, but the "real" v1-removal end state).
- Note `SessionProjectedMessage` (session_query_service.rs:67-94) is the only place
  `SessionProjectedOperationPart`-style v1 shapes still cross the crate boundary; nothing
  else in `agena-web` / `agena-api-server` / `agena-api` consumes it.

---

## 8. Verify (prove the bridge is gone)

From the repo root:

```bash
# 1. The v1 projection symbols must have zero references anywhere (tests included):
grep -rn --include="*.rs" -E "messages_from_parts|assistant_message_from_run_parts|message_from_run|message_from_singleton|build_message|reserve_message_ids" crates/ || echo "BRIDGE GONE"

# 2. Crate compiles (runtime-session + its dependents):
cargo check -p agena-runtime-session
cargo check -p agena-runtime-session-core

# 3. Downstream crates that crossed the bridge still compile:
cargo check -p agena-application
cargo check -p agena-cli
cargo check -p agena-runtime

# 4. The processor's parts-only accumulator holds: no `Message` in processor/run|parts|tool_calls|helpers:
grep -rn "Message" crates/agena-runtime-session/src/session/processor/ || echo "PROCESSOR PARTS-ONLY"

# 5. Tests (migrate bridge usages in store.rs:1714, manager/tests.rs, prompt_window.rs, processor.rs first):
cargo test -p agena-runtime-session
cargo test -p agena-runtime-session-core

# 6. Cross-crate integration:
cargo test -p agena-application
cargo test -p agena-cli
```

Definition-of-done checks:
- `grep` above is empty for the bridge symbols.
- `processor/` contains no `crate::message::Message` production usage.
- `prompt_window.rs`, `manager/compact.rs`, `manager/history.rs`, `manager/replies/replies_execution.rs`,
  `session/doom_loop.rs`, `session/cost.rs` have no `Vec<Message>` / `&[Message]` production signatures.
- `store.rs` no longer exports `messages_from_parts`; `session_from_view` doc updated.
- `list_projected_messages` is parts-native (option a) or replaced (option b); `agena-cli`
  and `agena-application` compile unchanged.
- No `.agena/tui-app` files touched (parallel agent's work).
