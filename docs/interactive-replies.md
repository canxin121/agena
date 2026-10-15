# Rich replies, backend demos, and terminal rendering

## What was investigated

The public ChatGPT component architecture is documented in the [Apps SDK UI guide](https://developers.openai.com/plugins/build/chatgpt-ui) and [component bridge reference](https://developers.openai.com/plugins/reference#windowopenai-component-bridge). A tool can associate a `text/html;profile=mcp-app` resource with its result through `_meta.ui.resourceUri`. ChatGPT hosts the component in an iframe; MCP Apps uses a `postMessage` JSON-RPC bridge for initialization, tool input/results, tool calls, messages, and model context. `window.openai` adds ChatGPT-specific state and display APIs. Backend data remains authoritative; component state holds temporary UI choices. The exact internal implementation behind a particular ChatGPT screenshot is not publicly established by these documents.

Agena now uses that useful separation—reply component, bounded host bridge, persistent UI state, authoritative backend—on top of its existing preview infrastructure. **`agena-ui` is an Agena protocol, not a claim of MCP Apps or `window.openai` compatibility.** It does not expose the host's tool gateway.

The terminal investigation covered:

| Project                                                                                                 | Relevant source                                                                                                 | Practical finding                                                                                                                                         |
| ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [Oh My Pi](https://github.com/can1357/oh-my-pi)                                                         | `packages/tui/src/components/markdown.ts`, `charts/chart-plan.ts`, `charts/chart-svg.ts`, `chat/table-chart.ts` | Structured chart plans produce SVG/native terminal image nodes while retaining textual data; memoized image and Markdown work avoids redundant rendering. |
| [OpenCode](https://github.com/anomalyco/opencode)                                                       | `packages/tui/src/plugin/markdown.ts`                                                                           | OpenTUI code-block renderer extensions and stable renderer identities let rich blocks participate in streamed Markdown.                                   |
| [Codex](https://github.com/openai/codex/tree/3df96b085c666b33d3775aa4e77219ec1ed828a0/codex-rs/mermaid) | `codex-rs/mermaid`, `tui/src/markdown_render/mermaid.rs`                                                        | A bounded terminal-native Mermaid parser/layout renderer returns semantic spans; unsupported or oversized diagrams retain source.                         |

Agena already had rich tables, math, syntax highlighting, images, semantic selection/copy, and themes. It now adds numeric charts in both clients and actual Mermaid relationship rendering in terminals without requiring image-protocol support. The Codex renderer is vendored under `third_party/codex-mermaid` at the immutable revision above. Its Apache-2.0 LICENSE, NOTICE, snapshots and provenance are retained; Cargo metadata is adapted for standalone use.

## An interactive reply

Only assistant text and assistant answer resources enable live apps. User messages, quoted examples, nested fences, ordinary file Markdown and unfinished streaming fences stay source. Malformed manifests show their original source and a useful validation message. Charts are safe structured data and can render in ordinary Markdown.

Use a completed, top-level fence:

````markdown
```agena-ui
{"id":"counter","title":"Counter","height":240,"html":"<button id='count'>0</button><script>agena.ready.then(s=>{let n=s?.n||0;const b=document.getElementById('count');b.textContent=n;b.onclick=async()=>{b.textContent=++n;await agena.setState({n})}})</script>"}
```
````

The card has source, reload, fullscreen, open-in-preview, service status, run/stop and logs. App identity and UI state belong to `(conversation session id, app id)`, so unrelated tabs/directories do not share an app accidentally. App services use the **conversation's own workspace**, independent of whichever directory another pane currently selects. Completed cards retain their iframe while later reply text streams in.

The component runs in an opaque-origin iframe with scripts and JS-handled forms enabled. CSP blocks direct network, external scripts, nested frames and native form navigation. It receives no host cookies, API tokens, storage or DOM access. Inline HTML/CSS/JS and data images are supported. External imports, arbitrary tool calls, arbitrary URL fetches, binary/streaming responses and a complete MCP Apps protocol are outside this version's contract.

The bridge validates iframe identity and a per-document channel token. Requests can only reach a declared managed preview service, with GET/HEAD/POST/PUT/PATCH/DELETE/OPTIONS, Accept/Content-Type headers and string request bodies. Paths cannot escape their service; responses expose only body, status and content type. Request/response sizes and concurrency are bounded; reload/unmount cancels bridge requests.

```js
await agena.ready // restored widget state (or null)
agena.getState() // current local widget state
await agena.setState({ filter: 'open' }) // persists up to 64 KiB in Agena's DB
const response = await agena.fetch('api', '/tasks', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ title: 'A real task' }),
})
if (!response.ok) throw new Error(await response.text())
const task = await response.json()
```

`agena.locale` and `agena.theme` reflect host updates; `agena:environment` is dispatched when they change. A component should keep authoritative business data in its backend; widget state is for filters, selection and other presentation choices.

Within one iframe, `setState` snapshots JSON values and persists calls in order, including rapid UI events that do not await each previous call. Await the returned promise before navigating away when the final write must be durable. Different app views share persisted state; simultaneous edits from different views use the last completed save.

## Real backend services

Create backend files in the workspace using normal tools, then add services:

```json
{
  "name": "api",
  "command": "python3",
  "args": ["server.py", "--port", "{port}"],
  "cwd": "examples/interactive-app",
  "port": 0,
  "healthPath": "/health"
}
```

Parsing or saving a reply does **not** execute commands. **Run app** is the explicit start action. Commands and exact argv are visible in the card. Up to four services are started in manifest order. `cwd` must resolve inside the owning conversation workspace, including symlink resolution. Zero/omitted port allocates a free loopback port; `{port}` is substituted in argv without a shell. The command must bind that port on `127.0.0.1`. An occupied explicitly requested port is an error, never an implicit attachment to another process.

Each service reuses the preview registry, runtime, logs and HTTP/WebSocket proxy. Repeated Run clicks across panes are serialized. A service must respond over HTTP within 15 seconds (status below 500); early exits and timeouts surface errors. Failure rolls back services newly started by that operation. Stop terminates owned process groups so dev-server child processes are included. Registry changes share the existing live preview stream. Services remain manageable from the ordinary preview sidebar. Updating a running service's configuration requires stopping it first.

The authenticated workbench API lives at `/api/v1/workbench/apps/{owner}/{id}`: GET/PUT manifest, POST `/run` and `/stop`, PUT `/state`, GET `/logs`. Manifests and UI state persist in server SQLite. Read-only logs are bounded to the last 32 KiB per service. The proxy preserves known request lengths for servers that do not accept chunked requests, and strips host credentials and response cookies.

See [the complete Flowboard example](../examples/interactive-app/README.md) for a responsive task UI with real Python/SQLite CRUD and persistent data across restarts.

## Numeric charts

````markdown
```agena-chart
{"type":"line","title":"API requests","xLabel":"Day","yLabel":"Requests","labels":["Mon","Tue","Wed"],"series":[{"name":"API","values":[14,null,22]},{"name":"Cached","values":[8,12,16]}]}
```
````

Types are `line`, `area`, `scatter`, `bar`, and `pie`. Up to eight named series and 512 labels are accepted; every series has exactly one finite number or null per label. Nulls preserve gaps. Optional title/xLabel/yLabel describe the axes. Pie requires one nonnegative series with a positive total. Values above absolute `1e100` are rejected to keep layout arithmetic finite.

Web renders responsive SVG, hover values, clickable series legends, an accessible data table, original JSON and SVG download. Terminal line/area/scatter use theme-colored braille plots; bar and pie use horizontal value/proportion bars, including signed baselines. Area uses the same line trace in the terminal. Very long bar datasets show 24 rows per series with an explicit remaining count; full values remain in source/copy/export. Widths below 28 columns and invalid schemas fall back to source.

Terminal Mermaid supports flowchart directions TD/TB/BT/LR/RL, sequences, flat states, classes and ER relationships. Upstream limits are 16 KiB source, 16 nodes, 24 edges, 40-cell labels and 64 KiB canvas cells. Unsupported syntax or diagrams too wide for the current terminal show a reason and their original source. Completed fences only become diagrams. Native terminal graphics remain available for existing SVG/images/math.

## Validation and local reproduction

Routine checks:

```sh
cd packages/agena-web
bun test
bun run build
cd ../..
cargo test -p agena --bin agena server::preview
cargo test -p agena-runtime-contracts identity::tests
cargo test -p agena-tui-transcript
```

Browser regressions live in `packages/agena-web/tests/browser/`. They mount real production components and exercise real local API/database/Git operations. Use an isolated server/data directory on a different port from your running Agena instance, plus a Vite proxy to that port. For example, run `agena server --port 3228 --database-path /tmp/agena-ui/runtime.sqlite` with `AGENA_SERVER_DATA_DIR=/tmp/agena-ui/state`; configure the verification Vite server on 5179 to proxy `/api`, `/auth`, `/health` to 3228. Do not point these tests at a shared production server.

With Playwright Core and its browser installed, run:

```sh
AGENA_UI_TEST_URL=http://127.0.0.1:5179 node packages/agena-web/tests/browser/rich-replies.mjs
AGENA_UI_TEST_URL=http://127.0.0.1:5179 node packages/agena-web/tests/browser/workspace-tab-context.mjs
AGENA_UI_TEST_URL=http://127.0.0.1:5179 node packages/agena-web/tests/browser/git-sidebar.mjs
```

`AGENA_UI_TEST_BROWSER_MODULE` can point to an existing Playwright Core module, and `AGENA_UI_TEST_CHROME_PATH` to an installed Chrome executable. The tests create temporary state, use isolated Git repositories, stop their app services, and retain screenshots. Fixtures are development-only, outside the production build.

Validation for this change passed all 894 frontend tests, the production build, 11 preview/backend tests, three identity tests, three rich-transcript integration tests, and all 21 vendored Mermaid tests. Browser regressions covered real backend CRUD, persistence, sandbox boundaries, streamed reply identity, mobile/full preview, real Git staging/committing, branch/worktree/compare entry points, and tab context menus across split groups.

The full transcript unit suite has one existing failure, `renderer::tests::a_pending_interaction_is_never_hidden_by_newer_siblings` (167 pass, one fail). The same assertion was reproduced in a separate clean worktree at the pre-change revision `56cc6d7b4`; this change does not modify that behavior or its fixture.
