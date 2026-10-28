# Stateful + Live Inline Generative UI

This is an interactive reply runtime, not an installed React/Vue application.
The model describes a bounded component tree, data, typed local state, derived
expressions and local actions. The host renders its own components. The existing
`inline-ui` reference fence places the interface inside normal reply prose.

## Two independent update loops

**Reader interaction:** input → local typed state → derived expressions → visible
components. Filtering, switching views, selecting tasks and scenario parameters
never need another model call and never trigger an external tool automatically.

**Agent work:** `prepare` → emit the returned fence early → actual work → `update`
with `expected_revision` → authenticated SSE snapshot → existing components update.
The same reference is used throughout. Valid user choices and stable node focus are
retained. The UI phase is an explicit agent-authored status, not an inferred promise
that a real process or business operation has completed.

### Progressive tools

| Tool | Purpose |
| --- | --- |
| `agena.inline_ui.guide` | Overview, one content kind, or `layout` / `state` / `actions` / `live`. `panel` gives the exact schema and a complete dashboard example. |
| `agena.inline_ui.validate` | Check a document before publication; no UI state is created. |
| `agena.inline_ui.prepare` | Create a session-scoped live head at revision 1 and return its fenced embed. |
| `agena.inline_ui.read` | Read the current agent-authored document and revision; not the reader's private local input. |
| `agena.inline_ui.update` | Atomically replace the document or patch panel data/nodes, with optimistic revision checking. |

The model must include the returned fence in visible commentary before doing work
that should update the interface. Preparing data alone does not render a UI. It is
not necessary to repeat the fence in every update or duplicate it in the final answer.

## Panel contract

```json
{
  "version": 1,
  "title": "Scenario explorer",
  "blocks": [{
    "type": "panel",
    "id": "scenario",
    "program": {
      "root": "main",
      "state": {
        "quantity": {"kind":"number","initial":20,"min":0,"max":100,"step":1}
      },
      "data": {"unitPrice":12},
      "computed": {
        "total": {"$op":"mul","args":[{"$get":"state.quantity"},{"$get":"data.unitPrice"}]}
      },
      "actions": {
        "copy": [{"type":"copy","text":{"$op":"concat","args":["Illustrative total: ",{"$get":"computed.total"}]}}]
      },
      "nodes": [
        {"id":"main","kind":"card","props":{"title":"Illustrative calculation"},"children":["quantity","total","copy"]},
        {"id":"quantity","kind":"slider","bind":"quantity","props":{"label":"Quantity"}},
        {"id":"total","kind":"metric","props":{"label":"Total","value":{"$get":"computed.total"},"prefix":"$"}},
        {"id":"copy","kind":"button","action":"copy","props":{"label":"Copy current result"}}
      ]
    }
  }]
}
```

No generated code is executed. The flat node graph has a single root; node IDs and
state names remain stable across updates. Children can only occur once in the tree.

### Components

Layout: `stack`, `row`, `grid`, `card`, `details`.

Presentation: `text`, `heading`, `metric`, `badge`, `divider`, `progress`, `chart`,
`table`, `timeline`, `callout`.

Controls: `tabs`, `select`, `checkbox`, `switch`, `slider`, `number`, `input`,
`button`, `tasks`.

Properties are allowlisted per kind. Layout uses semantic spacing and tone tokens,
not arbitrary CSS. Grid columns are 1–4 and collapse responsively. Text remains
plain text; links/citations can be written in the surrounding ordinary Markdown.
Tables support up to 256 row objects, columns are `{key,label}`. Task/timeline
items are `{id,title,detail?}`. Chart series are `{name,values}`; missing values
are null, not zero. Empty initial chart datasets can be populated by live updates.

### State and expressions

State kinds: boolean, bounded number, text, choice, selection. Numeric fields require
min/max and may declare a step. Choice options have stable values and readable labels.
Selection contains IDs, not arbitrary executable objects. On a new spec, old valid
values survive; invalid or removed fields are reconciled to the declared defaults.

`$get` reads `state`, `data`, `computed`, or the current `item` in a collection operation.
`$op` uses a known operator and argument list. `$literal` escapes reserved expression
keys. Unknown operations, unsafe property names and computed dependency cycles reject
the document. Both clients use the same short-circuit and numerical semantics.

Arithmetic: add/sub/mul/div/min/max/round/clamp. Logic: eq/ne/lt/lte/gt/gte/and/or/not/if.
Collections: length/sum/average/filter/map/pluck/includes. Text: concat/format/coalesce.

There is no eval, arbitrary function, DOM lookup, network request, file access,
clock, random generator or implicit tool call. Division by zero is an error rather
than silently generating infinity. Explain assumptions and distinguish simulations
from measured data; an interactive model does not validate its own assumptions.

### Local actions

A button references a named sequence of `set`, `toggle`, `reset`, `copy` actions.
Input controls update only their bound declared field. Hidden/disabled controls
reject events. Action sequences are atomic: a failure leaves prior input untouched.
If a new valid data revision makes the current parameter combination invalid, the client shows an error rather than silently changing input. An explicit reset restores defaults (Web reset button, TUI Ctrl+R).
Clipboard effects are returned to the host and performed only for a user activation,
never during rendering, history replay, snapshot arrival or reconnection.

## Agent updates

```json
{
  "update": {
    "reference": "REFERENCE_FROM_PREPARE",
    "expected_revision": 1,
    "patch": {
      "block_id": "scenario",
      "data": {"unitPrice":15}
    },
    "phase": "complete"
  }
}
```

A panel patch merges named data fields and can upsert complete nodes by stable ID.
`remove` deletes a subtree and its parent edges. The root cannot be removed.
Use a complete `document` replacement when changing state declarations/computed
expressions/actions; it is mutually exclusive with `patch`. The entire candidate
is validated before commit. Failed edits never replace the last valid interface.

`expected_revision` is mandatory. Concurrent writers cannot silently overwrite one
another. A repeated call ID is idempotent. The tool returns the new revision; a
conflict requires read/reconcile/retry. Phases are working, complete and error.

The immutable prepare result remains in the normal tool history for authorization
and provenance. The current agent-authored head is stored in the existing private,
session-scoped plugin storage, using secure atomic rename and cross-process file
mutation locks. User-local state is deliberately not part of that head.

## Delivery and security

Session membership in the originating successful prepare invocation is checked
before reading or streaming a reference. A live head is never a global public
resource. The full opaque reference must match the prepared result.

```
GET /api/v1/sessions/{session}/inline-ui/{reference}
GET /api/v1/sessions/{session}/inline-ui/{reference}/live
GET /api/v1/sessions/{session}/inline-ui/{reference}/stream
```

The first route returns the current document for body/copy compatibility; `/live`
returns revision/phase/document. SSE emits complete validated `inline_ui` snapshots,
not a fragile sequence of patches the client must reconstruct. Subscribe-before-read
avoids missed commits. In-process commits notify immediately; a two-second reread
recovers cross-process updates and membership revocation. Headers prevent caching
and request that reverse proxies not buffer the stream. Deployment proxies must
still be configured to pass SSE normally.

Clients ignore stale/duplicate revisions, reconnect with backoff, and recover the
current snapshot. Bad snapshots do not replace a working view. Unavailable/forbidden
references clear the view. Source text and UTF-8 cursors remain unchanged; only the
read-time UI projection changes. No reference or input can execute an external action.

Budgets: 60 KiB document, 24 blocks, 128 nodes/panel, depth 12, 48 state fields,
64 computed fields/actions, 30,000 expression evaluation steps, 1,024 collection
items, 16 operations/action, 32 references per body. The live record has a 128 KiB
storage bound. These bounds apply to model-supplied structure, not only the renderer.

## Web UI

Responsive cards/grids, tabs, selectors, sliders, numeric/text input, checkbox task
lists, conditional sections, timeline, metrics, progress, charts and tables use the
host theme. Keyboard labels/focus, mobile widths and reduced motion are supported.
The main transcript owns the interface; no iframe or development server is required.

User state is bounded and scoped by server/session/reference/panel. It is retained
in tab-local memory and best-effort sessionStorage, not sent to the agent or shared
across browsers. Changes to the agent document preserve still-valid local values.
Copying the whole answer uses the current displayed snapshot and reader state.
`/ui` focuses the latest displayed interface (or a supplied reference).

## TUI

The main transcript displays the current revision's content and current selections.
`/ui [reference]` opens a native keyboard work surface for the latest/selected UI.
It is part of the terminal client, not a Web Preview. Both clients run the same
expression, validation and action rules.

Tab/Shift+Tab moves control focus; arrows change tabs/options and adjust numeric
values; Home/End selects numeric bounds; Space toggles checkbox/task selections;
Enter activates buttons or edits text/numbers; Escape cancels an edit or closes.
PageUp/PageDown scrolls content. `[` / `]` changes panel when a document has several.
Pasted text is accepted only inside an explicit editor. A control removed during
editing cancels that edit rather than writing into a different control.

At wide widths content and controls are side-by-side; narrow terminals stack them.
Live subscriptions update the existing transcript and dialog; local input survives
valid agent revisions. Charts have exact-data terminal tables rather than a fake
browser chart. The local clipboard action, whole-answer copy and transcript export
use current local selections. Terminal input is retained in the loaded session,
not synchronized to another client or persisted across a terminal restart.

## Acceptance fixtures and checks

`examples/inline-ui/live-dashboard.json` contains the 45-node, three-view dashboard:
channel filtering links cards/table/chart; task selection links overview progress;
budget and margin recompute simulation results; warnings/formulas render conditionally.
All financial data is explicitly illustrative. `live-update.json` is a patch example.

`program-cases.json` is generated by the Rust evaluator and checked by the Web tests
after every action, including exact semantic copy output. Regenerate it with:

```sh
cargo run -p agena-domain --example inline_ui_program_cases --offline > examples/inline-ui/program-cases.json
```

Focused checks:

```sh
cargo test -p agena-domain -p agena-runtime-plugins -p agena-bundled-plugins stateful_ui --offline
cargo test -p agena-tui-app --lib stateful_ui --offline
cd packages/agena-web
bun test ./tests/inlineUiProgram.test.ts
PLAYWRIGHT_MODULE=/path/to/playwright CHROME_BIN=/path/to/chrome bun run test:inline-ui:live
```

The isolated browser probe uses the real transcript renderer and an SSE fixture:
linked inputs, keyboard controls, live node/data replacement, stable focus, duplicate
and malformed snapshots, reconnection, copy, remount state and session isolation.
Screenshots/results are in ignored `artifacts/inline-ui/live-*`. Rust tests exercise
atomic concurrent writes, rollback and terminal keyboard/rendering paths. Tests do
not call a real model or make real business changes.

### Evaluated result budgets

The 30,000-operation budget is complemented by deterministic resolved-value
charges in both runtimes: one value is limited to 256 KiB and the cumulative
produced-value budget per evaluator is 4 MiB (UTF-8 strings/keys plus fixed node
charges). This prevents a small nested map or computed dependency graph from
amplifying a bounded document into an unbounded allocation. Invalid updates and
invalid user actions retain the last valid state rather than partially applying.
Repeated embeds share their scoped state while retaining unique DOM identifiers.

## Compact host-native presentation

Web presentation uses Agena's `--text-ui-label`, `--text-ui-header`, `--text-micro`,
`--padding-scale`, foreground/background, border, input and radius tokens. The
normal layout rhythm is 4/8/12px before the user's padding scale. Cards have a
10px inset by default, desktop inputs/buttons have a 32px minimum height, and
coarse-pointer controls keep a 40px hit area. Global panel styles are explicitly
scoped so similarly named classes in the host cannot inherit them.

Card boundaries are deliberate; grids no longer stretch short content to the
height of unrelated tall neighbors. Nested cards establish their own container
query width. Explicit one-column grids stay one column. Short metric grids can
remain two columns on narrow displays, while forms and complex sections stack.
Charts inside cards omit the extra frame/margins. Their SVG coordinates follow
the measured width: labels keep a readable size instead of shrinking from an
800px canvas. SVG export captures resolved theme colors and fonts.

The terminal work surface uses the existing modal frame, border and selection
styles. Header/footer rows are content-sized; controls consume only the rows they
need, not a fixed percentage of the window. A read-only result does not display
an empty controls pane. Preview facts/metrics are compact, meaningful sections
have separators, and input labels are not needlessly repeated in both panes.
These changes affect presentation, not the complete copy/export representation.
The preview scroll limit is recomputed after live updates so removed content
cannot leave the viewport stuck in empty space.

Run `bun run test:inline-ui:layout` with the same Playwright/Chrome environment as
the other probes. It checks all 24 controlled node kinds at 320–1240px, both themes,
large host font settings, RTL, nested grids, long Chinese/unbroken strings, style
isolation, keyboard disclosure, theme-preserving chart export and touch targets.
`probe-stateful-ui.cjs` records layout measurements alongside its live-state checks.
Set `AGENA_INLINE_UI_LAYOUT_ARTIFACTS` when running terminal `stateful_ui` tests to
save 40-, 80- and 120-column rendered buffers. Browser artifacts are ignored under
`artifacts/inline-ui/density-after`; no screenshots or tests connect to real user data.
