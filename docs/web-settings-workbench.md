# Web Settings Workbench

The Web Settings Workbench is the browser counterpart of the TUI Settings Studio. It intentionally keeps the same six top-level domains while using nested, searchable pages so a large configuration surface does not become one unstructured scrolling form.

## Design goals

1. **TUI parity without copying TUI limitations.** Every setting the TUI can persist has a Web editor or a safe advanced JSON-path escape hatch.
2. **Explicit configuration layers.** Global, Workspace, Session, and Effective values are never silently conflated. Effective values are read-only; writes always name their target layer.
3. **Server-owned validation.** The browser may provide structured controls, but the server validates the complete composed configuration before persistence and owns runtime reloads.
4. **Discoverable hierarchy.** Dense domains use a reusable section workbench with page search, URL deep links (`?view=`), responsive page selection, and per-domain remembered pages.
5. **Preserve unknown data.** Provider models, plugin configuration, permission documents, and harness records retain fields that are not represented by the current structured form. Raw JSON remains available where the schema is open-ended.
6. **Safe source inspection.** Editors show Effective, Global, and Workspace values together. Security-sensitive advanced editing is clearly marked and never guesses a write target.

## Information architecture

### Models & Providers

- **Provider Studio** — create or delete providers; configure authentication, interactive OAuth, timeout policy, adapters, live model discovery, manual models, and per-model metadata.
- **Model defaults** — select a complete provider / adapter / model identity plus thinking, speed, verbosity, and parallel-tool-call modes for the runtime default and automatic approval model.
- **Model Catalog** — server-side search, origin filtering, paging, source refresh, and full capability / mode / pricing inspection.
- **Configured inventory** — read the configured provider, adapter, endpoint, and model topology.

### Permissions

- **Permission Studio** — edit Global, Workspace, current Session, or read-only Effective permission documents. The editor covers filesystem defaults and path rules; network zones and domain rules; and the default tool policy, tool-name rules, and shell command rules. Every built-in default is an ordinary entry inside those collections: the temporary and managed project-state directories are `path.rules` entries, the interaction and web tools are `tools.names` entries, and the command classes (`no-op`, `routine`, `dangerous`) and the read-only tool class are `tools.rules` entries. Writing an entry of the same name replaces the built-in one and deleting your entry restores it, so `auto` written over a built-in hands that class back to the approval model. All rule types support create, rename, mode changes, and delete. Raw `PermissionConfig` JSON remains available.
- **Persistent rules** — inspect and revoke durable approval rules created by interactive permission decisions.

The source selector shows a compact summary for every loaded layer so the user can see whether a decision comes from Global, Workspace, Session, or the merged Effective policy before editing.

### Plugins & Tools

- **Plugin Workbench** — searchable/filterable plugin list with Overview, Config, Tools, Commands, Views, Controls, Capabilities, Logs, and Diagnostics tabs.
  - Plugin Config materializes JSON Schema defaults, applies localized schema overlays, renders nested objects/arrays/enums/unions, supports array reorder/copy/delete, and falls back to raw JSON for open-ended structures.
  - Saves contain the minimal plugin-owned override rather than a copy of every schema default.
  - Dry-run validation and the saved/draft override diff are available before a runtime-reloading save.
  - Manifest tools can be invoked in the active Session with JSON input while their schemas and declared tags remain visible.
- **MCP Server** — listener enablement, authentication mode, mixed-auth anonymous access, OAuth client registration, public resource URL, issuer URL, OAuth password, endpoint inspection, and tool exposure.
- **Tool harnesses** — named Browser, Shell, and Editor harnesses with explicit Global/Workspace targets, effective-value copying, rename/delete, raw JSON, browser launch options, shell environment variables, and all typed runtime fields.

#### Bundled web search configuration

In **Plugin Workbench → agena.web → Config → Search**, choose the backend used when `web.search` omits `engine` or sets it to `auto`. These are plugin-owned settings; the examples below are the contents of `agena.web`'s settings object.

| `search.provider` | Default endpoint | Server credential environment variable | Behavior |
| --- | --- | --- | --- |
| `html` (default) | DuckDuckGo, Bing, Baidu | None | Existing HTML engines, in that fallback order |
| `brave` | `https://api.search.brave.com/res/v1/web/search` | `BRAVE_SEARCH_API_KEY` | Structured web results; at most 20 per request |
| `tavily` | `https://api.tavily.com/search` | `TAVILY_API_KEY` | Basic search, without generated answers or raw page bodies; reports returned credit usage |
| `exa` | `https://api.exa.ai/search` | `EXA_API_KEY` | Automatic search with highlights; reports returned total cost |
| `searxng` | Required: your instance's full `/search` URL | None | JSON must be enabled on the instance; reports unresponsive upstreams |

For a hosted API, set the credential in the Agena server's environment and configure:

```json
{
  "search": {
    "provider": "brave",
    "default_limit": 5,
    "max_limit": 20
  }
}
```

`search.api_key_env` can select a different environment variable **name**. Never put the key itself in plugin settings. `search.endpoint` can select a compatible HTTPS API endpoint; it cannot contain URL credentials, query parameters or a fragment. The configured endpoint receives the API credential and search query.

For a private SearXNG service:

```json
{
  "search": {
    "provider": "searxng",
    "endpoint": "http://127.0.0.1:8080/search",
    "allow_private_endpoint": true
  }
}
```

The runtime network policy must also allow this endpoint. The private-endpoint setting applies only to the configured SearXNG search service; it does not enable private page fetching. Search connections pin the approved DNS addresses, use direct connections and reject redirects. An HTTP proxy configured in the process environment is not used by these API adapters; configure a compatible HTTPS endpoint if a gateway is required.

Explicit `engine: "bing"`, `"duckduckgo"` or `"baidu"` selects that HTML engine. A configured API's error or empty result does not automatically send the query to another provider. API requests are not retried automatically. HTTP status and numeric `Retry-After` are reported without echoing response error bodies.

Use bare hostnames in `allowed_domains` and `blocked_domains`; subdomains match and exclusions win. Filters apply locally to every provider and are also sent to Tavily/Exa. Local filtering can leave fewer results than requested. API results include the effective limit, provider/filtered counts, partial/truncated flags and bounded snippets; fetch relevant pages before relying on their contents. Responses are limited to 4 MiB, each title to 512 characters and each description to 2,000 characters. These adapters have deterministic HTTP-fixture tests; no live paid-provider relevance or latency comparison is claimed.

#### Page transport and optional HTML extraction

Ordinary `web.fetch`/`web.crawl` requests use bounded HTTP streaming. Each initial request, redirect and robots request checks runtime network permission and public DNS addresses before connecting; the connection pins those addresses. Environment proxies and automatic retries are disabled for this path. The configured request deadline covers robots, DNS, per-host pacing, redirects and body streaming. Redirects stop after ten hops; robots bodies stop at 512 KiB. `fetch.request.max_body_bytes` has a 32 MiB ceiling and `timeout_secs` a 120-second ceiling. Declared HTTP character sets, including GBK, are decoded explicitly; replacement characters produce a warning.

The supplied HTTP scheme and query parameters are preserved, including order and repeated keys. `final_url` reports the transport destination independently of an HTML canonical hint. Relative links resolve against that destination or the document's `<base>`. URL credentials are rejected. PDF/binary responses should be downloaded explicitly and read with a suitable document tool.

Readability remains the default extractor. Set `fetch.extractor` to `trafilatura`, or pass `extractor: "trafilatura"` to one `web.fetch` call, to use an installed local alternative. Prepare Python separately, for example `uv venv /path/to/extract-env` then `uv pip install --python /path/to/extract-env/bin/python trafilatura==2.3.0`, and set `AGENA_EXTRACT_PYTHON=/path/to/extract-env/bin/python`. Runtime calls do not install dependencies or send HTML to a hosted service. The converter receives HTML through stdin and does not fetch the URL itself. A missing package or failed extraction is reported without an automatic backend retry.

Two converters may run at once, with a 20-second process timeout (25 seconds including admission/startup), a 32 MiB HTML source limit and 2 MiB extracted-text limit. The small authored article/documentation/forum comparison retained all markers with readability; Trafilatura omitted a forum heading and repeated part of an answer. This supports keeping it optional, not a general quality or speed ranking. The fixtures and observed output are in `docs/research/tool-modernization-extraction-fixtures.json`.

The standalone fetch result limits Markdown to 16,000 characters and links to 32; `available_markdown_bytes`, `available_link_count`, `output_truncated`, and warnings distinguish the preview from the available extraction. A per-call extractor override bypasses the configured page cache. HTTP errors and truncated bodies do not become cache entries. Crawl attempts (including failed requests and cache hits), depth, and URL discovery have independent bounds. Cached crawl documents recheck current permission for the requested and actual transport URLs; older entries without a final URL are fetched again. In-memory cache eviction uses a weighted 64 MiB budget, which is not a strict bound on all process allocations. Metadata handles are shared only while their owning stores remain alive.

Rendered fetches now use a separate disposable context on the managed browser and the same native CDP request checks as interactive operations. The attached page's intercepted document and HTTP subresource requests require current host permission and public DNS validation before continuation. Robots rules are checked for document destinations. Service workers and the context's download behavior are disabled. Each fetch owns its root connection with `disposeOnDetach`, so success, failure, timeout and caller cancellation release that context. At most two renders are admitted; launch, readiness, requests and extraction share the configured deadline.

The captured DOM obeys the configured UTF-8 byte limit; observed resource data also has a four-times-body budget. This is not a strict Chromium memory/network bound. Chromium resolves connections itself, and the attached-page checks are not a complete sandbox for browser networking, worker targets, WebSockets or WebRTC. The ordinary HTTP DNS-pinning guarantee does not apply here. Local fixtures validate JavaScript-generated text, blocked redirects and subresources before dispatch, actual HTTP status, Unicode clipping, and context cleanup on timeout/cancellation. Current host callback authority is explicitly carried into CDP authorization workers and refreshed by foreground interactions; expired authority is rejected by the host.

#### Optional Playwright browser interactions

Set `browser.interaction_backend` to `playwright` to use Playwright's actionability checks for `browser_click`, `browser_type`, and `browser_wait`. The default is `native`. Set `AGENA_BROWSER_PYTHON` in the Agena server environment to an existing Python interpreter with the `playwright` package installed. For example, prepare a dedicated environment with `uv venv /path/to/browser-env` and `uv pip install --python /path/to/browser-env/bin/python playwright==1.58.0`, then set `AGENA_BROWSER_PYTHON=/path/to/browser-env/bin/python`. Runtime calls never install dependencies. This adapter connects to Agena's managed Chrome/Chromium, so a Playwright-managed browser download is unnecessary.

The adapter uses strict CSS locators or a checked reference from the latest Agena snapshot. Clicks wait for Playwright's visible, stable, enabled, and event-receiving conditions; fills use native Playwright input/Enter behavior; selector waits require visibility. Native selector waits retain their existing presence semantics. Click/type accept `timeout_ms` (default 30,000, range 1–120,000). The bridge allows five additional seconds for process startup/admission. A missing dependency or failed Playwright action is reported without silently retrying through another backend, since an action may already have changed the page.

The existing native connection continues to own each session's browser context, navigation interception, sensitive-value omission, snapshots, screenshots, downloads, and shutdown. Playwright receives only the owned target/context IDs and performs one action per short-lived connection. The bridge is serialized and excludes native download transfers while attaching. It disconnects after the action without closing Agena's browser contexts. An optional `frame_selector` selects one CSS iframe for click/fill/wait; inside that frame use CSS selectors. Snapshot references remain local to the main document and cannot be combined with `frame_selector`. Browser tracing, arbitrary JavaScript tools, and a separate Playwright MCP service remain separate extension choices.

Requests travel through stdin instead of command-line arguments. Debug logging and Node injection variables are removed from the bridge environment; raw exception/call-log text is omitted because it can contain filled values. This prevents diagnostics from echoing input, but a page can still reflect submitted content into ordinary visible text. Existing snapshot omission is not a guarantee that a hostile page cannot reveal user input.

Playwright 1.58.0 with the installed Chrome passed local fixtures for covered clicks, Unicode fill, sensitive-value omission, snapshot references, delayed visibility, iframe input and strict selectors, context retention and scope, and continued native navigation interception. This is a correctness test of the optional adapter, not a latency or general website-success benchmark. Browser subprocesses now use the shared Unix process-session / Windows Job Object policy; active actions hold an idle lease, and idle expiry checks/removes the browser under one lock. Native download cleanup retains its action gate even if the caller cancels.

### Runtime & Session

- **Provider client versions** — edit exact Codex, Claude, and Gemini compatibility versions or refresh all three from npm.
- **Session compaction** — configure automatic compaction and reserved tokens, with layer source display and clear-override actions.

### Interface

- **TUI preferences** — server-backed TUI locale, color scheme, graphics mode, plugin theme, default activity expansion, plugin-contributed activity kinds, and exact tool expansion overrides. Long dynamic catalogs are searchable.
- **Web appearance** — browser-only locale, theme, fonts, density, padding, radius, and composer geometry.
- **Conversation display** — browser-only timestamps, reasoning visibility, idle collapse, activity-kind expansion, and exact tool expansion overrides.

The server TUI locale and browser Web locale are deliberately independent.

### Diagnostics

- **Runtime & tracing** — tracing levels, runtime generation, providers/plugins, configuration source layers, resolved configuration, validation, and runtime reload.
- **Advanced settings** — edit any explicit JSON path in the Global or Workspace layer, compare it with Effective/other-layer values, dry-run validate, save with reload, or clear the selected override. This is the completeness escape hatch for settings without a dedicated form.
- **Activity history**, **Memories**, and **Usage** — operational inspection pages kept separate from configuration editing.

## Persistence and reload semantics

- Normal server settings use `/api/v1/settings` or `/api/v1/settings/layers/{global|workspace}` with `validate=true` and `reload=true`.
- Effective settings are never writable.
- Session permission writes use `/api/v1/sessions/{session_id}/permission`.
- Provider Studio and MCP use their dedicated server control APIs.
- Model Catalog refresh and provider client-version refresh use their dedicated background operations.
- Plugin settings save the full configured-plugin record at the quoted path `plugins.list."<plugin-id>"`, preserving package and timeout fields while replacing only the plugin-owned minimal `settings` override and enabled state.
- Empty/inherited values are removed from the selected layer instead of being persisted as meaningless empty strings where the setting contract supports inheritance.

## Safety model

- Provider secrets and MCP OAuth passwords remain in dedicated editors.
- The advanced path editor warns that configuration may contain credentials and requires an explicit layer and path.
- Every advanced/plugin write can be dry-run validated against the complete composed runtime configuration.
- Destructive clear/delete actions require explicit user interaction; high-impact layer clears use confirmation dialogs.
- Raw JSON editors preserve access to open-set configuration while server validation remains authoritative.

## Verification

The Web package is expected to pass:

```sh
bun run check:imports
bun run typecheck
bun test
bun run build
```

Focused contract and unit tests cover the section hierarchy/deep links, advanced layer editor, model verbosity metadata, plugin schema defaults/overrides/unions, plugin settings workflow, permission layer summaries, TUI/Web locale separation, and dynamic Interface searches.
