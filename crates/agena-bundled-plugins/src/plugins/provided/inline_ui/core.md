# Inline Generative UI — complete first-use guide

**Fast path:** `guide()` once when learning → `prepare({document,phase?})` → paste the exact returned `embed` into a normal assistant reply. Once this contract is known, call `prepare` directly. After new real data arrives, call `update({update:{reference,expected_revision,patch?...}})` to refresh that *same* embed. Never call a separate validator first. A failed prepare/update returns a located error; correct the input and retry. `read({reference})` is for recovery/inspection/conflict reconciliation, **not** required before routine updates.

## Publishing is not automatic

`prepare` returns `reference`, `embed`, `revision:1`, `phase`; it does *not* choose where the content appears. Place `embed` unchanged between Markdown paragraphs (never inside a quote, list or outer fence). Its entire content is a fence with **only** the tool-returned ID:

````markdown
Here are the interactive figures:

```inline-ui
REFERENCE_RETURNED_BY_PREPARE
```

The assumptions and citations follow below.
````

The ID above is illustrative; **never invent an ID**. Do not place document JSON inside the fence, use `[[inline-ui:...]]`, `agena-ui` JSON fences, old present calls, or a separate app. For ongoing work, show the embed early in the visible reply, then update the existing reference; do not embed it a second time at the end.

## Two distinct layers — do not mix their fields

`document = {"version":1,"title":"optional","blocks":[...]}`. Blocks are **ordered top-level content**, each with a unique `id` and `type`. Top-level static blocks are `paragraph` (`text`), `callout` (`title?,tone?,text`), `steps` (`title?,items`), `comparison` (`title?,columns,rows`), `metrics` (`title?,items`), `table` (`title?,columns,rows`), `details` (`title,text`), `chart` (`title,chart_type,labels,series,x_label?,y_label?`). These are **not** `program.nodes`. Top-level table rows are string arrays; panel table rows are objects. Use `guide({sections:["chart"]})` for static chart shapes and supplementary schemas.

An interactive interface is one top-level block `{"type":"panel","id":"scenario","program":{...}}`. A program has `root` (a node **ID**, not an embedded node), a **flat** `nodes` array, optional `data`, `state`, `computed` and `actions`. Each node: `{"id":"stable-id","kind":"metric","props":{...},"children":[],"bind":..., "visible":..., "action":...}` with optional fields omitted. Container `children` are arrays of **ID strings**, not nested objects. Every node must be reachable exactly once from root; no duplicate IDs, shared or cyclic branches or dangling IDs; depth ≤12, nodes ≤128. Keep IDs stable across revisions so client-local state, control focus and incremental layout survive.

## Data, state, computed and actions

- `program.data`: agent-authored facts, e.g. `{"base":120,"rows":[...]}`. Agent updates may patch this. Label invented figures as illustrative; cite real sources in surrounding prose.
- `program.state`: **client-local user input declarations**, not model-observed data. `{"flag":{"kind":"boolean","initial":false},"amount":{"kind":"number","initial":25,"min":0,"max":100,"step":5},"note":{"kind":"text","initial":"","max_length":120},"mode":{"kind":"choice","initial":"all","options":[{"value":"all","label":"All"},{"value":"paid","label":"Paid"}]},"checked":{"kind":"selection","initial":[]}}`. Numeric state requires `min`, `max`; `step` optional. `choice` needs valid `options`; selection is an array of unique safe IDs. Controls reference keys with **top-level node** `bind:"amount"` (not an expression or `props.bind`).
- `program.computed`: named expression trees derived from agent data and current local state; evaluated by both clients. Example `{"total":{"$op":"mul","args":[{"$get":"data.base"},{"$get":"state.amount"}]}}`; a node property can use `{"$get":"computed.total"}`. Computed references must not cycle, including hidden branches.
- `program.actions`: named ordered lists of **local only** actions. A button has top-level `action:"resetForm"` and corresponding `"resetForm":[{"type":"reset"}]`. `set`: `{"type":"set","key":"mode","value":"all"}`; `toggle`: `{"type":"toggle","key":"flag"}` (boolean); `reset`: `{"type":"reset"}` (all local state defaults); `copy`: `{"type":"copy","text":{"$get":"computed.summary"}}` (text). Up to 16 actions per list. Invalid edits/actions are rejected atomically, no partial state or copy. Buttons do not run shell, tools, files, or network.

**Crucial:** the model cannot read the user's local slider/checkbox/input choices through `read` or `update`; they are never automatically sent back. A task checked in a UI is a local selection, **not** proof the Agent actually performed a task. `phase` reflects observed Agent work only, never inferred from progress bars or task nodes.

## Expression syntax that actually evaluates

A literal number, string, boolean, array or object is valid. A lookup is **exactly** `{"$get":"data.base"}`, `{"$get":"state.amount"}`, `{"$get":"computed.total"}`, or inside map/filter `{"$get":"item.price"}`; the root must be `data|state|computed|item` followed by safe dot-separated keys. An operation is **exactly** `{"$op":"add","args":[1,2]}` (no other keys); `{"$literal":{...}}` prevents objects containing reserved `$` keys from being interpreted.

Examples: `{"$op":"gt","args":[{"$get":"computed.total"},150]}`; `{"$op":"if","args":[{"$get":"state.flag"},"Yes","No"]}`; `{"$op":"sum","args":[{"$op":"pluck","args":[{"$get":"data.rows"},"price"]}]}`; `{"$op":"map","args":[{"$get":"data.rows"},{"$get":"item.price"}]}`; `{"$op":"filter","args":[{"$get":"data.rows"},{"$op":"gt","args":[{"$get":"item.price"},0]}]}`. `sum` and `average` accept **one array**, not variadic numbers; `map`/`filter` take **two** args (array, per-item expression). `visible` is a **top-level node field** with a boolean expression, e.g. `"visible":{"$get":"computed.high"}`; false hides the whole subtree and disables its interaction. Properties can be expression trees; keep labels/text and evaluated prop types valid.

`if`, `and`, `or`, `coalesce` short-circuit; guard division by zero, missing values and unsafe types. `round` and `format` accept a value plus optional digits 0..6. `clamp` has 3 args, `not` 1, comparisons 2, `concat` variadic. No JavaScript, callbacks, eval, fetch, templates, dates or randomness. Full operator names below are generated from the validator. Complex expression guidance is in `guide({sections:["expressions"]})`.

## Panel components and allowed properties

Node `kind` and `props` keys are **strict whitelists** (generated from `UiKind::allowed_props` below). Common shapes:

- Containers: `stack|row|grid|card` with `children:["id",...]`, `gap:"sm"|"md"|"lg"`, semantic `tone`; `grid` has `columns:1..4` as a maximum, **not fixed width**. `details` is a container with `title`.
- Text: `heading|text` with `text`, optional `size:"sm"|"md"|"lg"`, `tone`. `metric`: `label`, `value`, optional `prefix,suffix,detail,trend,tone`; `badge`: `text,tone`; `divider`: no props; `callout`: `title,text,tone`.
- Controls: `tabs|select` bind `choice`; `checkbox|switch` bind `boolean`; `slider|number` bind bounded `number`; `input` binds `text`; `tasks` binds `selection`. Controls need `label` (except tasks), optional `description,disabled`; tasks use `title,items:[{id,title,detail?}],empty`. `button` needs `props.label` + **top-level** `action`.
- Data: `progress`: `label,value,max,detail,tone`; `chart`: `title,chart_type:"line"|"bar"|"area"|"pie",labels:[...],series:[{name,values:[...]}],x_label,y_label`; `table`: `title,columns:[{key,label}],rows:[{key:value,...}],empty`; `timeline`: `title,items:[{id,title,detail?}]`. Charts/tables may bind computed arrays of matching shapes. Chart series lengths must match labels; pie uses one nonnegative series with positive total. TUI and Web share the same computed values.

**Whitelisted kind → properties (from the authoritative Rust code):**

{COMPONENT_CATALOG}

**Operator names (from the Rust validator):** {OPERATORS}

## Small complete working panel

The JSON below is a real `prepare.document` example, using illustrative values (7 flat nodes). Baseline `data.base=120` → slider `state.percent=25` → computed `total=150` → metric, conditional warning at values over 150, and local reset. Web and TUI evaluate identical state/computed semantics; the example is tested against actual validators and both evaluator implementations.

```json
{EXAMPLE}
```

After prepare returns `reference` and `revision:1`, the following is the **update tool input** using that returned reference, not a new document. It changes agent-authored baseline data only, preserving valid local percent input:

```json
{UPDATE}
```

Do not send `REFERENCE_FROM_PREPARE` literally. `update` returns the **new revision** and phase; use that revision directly as the next `expected_revision`. `read` is only needed if you lost the revision, want an inspection, or receive a conflict. On `revision_conflict`, read the current snapshot, **reconcile** concurrent changes, then submit a deliberate new update; never blindly replace the number.

## Phase, limits and safety

`prepare({document})` atomically creates `revision:1, phase:"complete"` for finished content; `prepare({document,phase:"working"})` creates a live work panel without an extra update. `update({update:{reference,expected_revision,patch:{block_id,data?,nodes?,remove?},phase?}})` atomically upserts **whole** nodes, patches data or deletes subtrees; use `document` **instead** of `patch` for a complete replacement. A phase-only update is permitted. For genuine agent failures use `phase:"error"`. Conflicts and invalid updates apply nothing. Do not assume an uncertain I/O failure is safe to blindly retry or that a new invocation is idempotent.

Document v1: **1–24** top-level blocks, **60 KiB** serialized maximum, unique ASCII IDs (1–80 chars), no control chars, at most 32 references per answer. Panel: **128 nodes**, depth 12; at most 48 state, 64 data/computed/action keys, at most 1024 array items, expression depth 24, 30,000 evaluation steps and bounded results. Text/labels, chart series and data rows have additional schema limits. Unknown fields/props/operators and unsafe inputs reject instead of being silently coerced.

Web/TUI own themes, font size, responsive density, keyboard/mouse focus, copy/export and terminal width; do **not** specify pixel widths, CSS, fake columns using spaces, empty placeholder nodes, or decorative spacer cards. Keep factual claims, units and uncertainty honest; progress must reflect actual observations. Local controls cannot perform arbitrary external actions. For complex chart shapes, full JSON Schema, 45-node dashboard and advanced updates, fetch only the needed `guide({sections:[...]})` sections (batch names; no core replay).
