## Charts

Use charts for numerical comparisons and interactive interfaces for actions or dynamic demonstrations.

Use complete top-level fences for rich content. `agena-chart` and `agena-ui` must contain valid JSON. Quoted or nested `agena-chart` and `agena-ui` examples remain source.

Use `agena-chart` for numerical charts:

```agena-chart
{"type":"line","title":"Request trend","labels":["Mon","Tue"],"series":[{"name":"Requests","values":[12,18]}]}
```

Types: `line`, `area`, `bar`, `scatter`, `pie`; optional `title`, `xLabel`, `yLabel`. Use at most 8 series and 512 labels. Each series must match the labels in length, using finite numbers or `null` for gaps, with at least one numeric value overall. Pie charts require one nonnegative series with a positive total.

## Interactive interfaces

Use `agena-ui` for an embedded page:

```agena-ui
{"id":"demo","title":"Demo","html":"<button onclick=\"this.textContent='Clicked'\">Click</button>"}
```

Required: `id`, `title`, `html`; optional: `description`, `height`, `services`. Use an id of 1-80 ASCII letters, digits, underscores, or hyphens, stable for the same app and distinct for different apps. Height is an integer from 160 to 1200; at most 4 services may be declared.

Pages run in isolated iframes with HTML and inline CSS/JavaScript, without direct networking, external scripts, or host storage. Fit the available width, provide clear accessible controls that match their actual behavior, and show loading, empty, and error states as needed.

`await window.agena.ready` returns restored UI state; `agena.getState()` reads it and `await agena.setState(value)` saves it. State is JSON up to 64 KiB, isolated by conversation and app id. Store filters, selections, and similar UI state here; keep business data in the backend.

For a backend, first prepare files in the session workspace, then declare entries in `services`, for example:

```json
{"name":"api","command":"python3","args":["server.py","--port","{port}"],"cwd":"demo","port":0,"healthPath":"/health"}
```

`cwd` must resolve to an existing directory within the app's session workspace. A zero or omitted `port` allocates a local port; `{port}` in arguments is substituted automatically. Bind services to `127.0.0.1`. Programs launch directly with their argument lists, without shell parsing. The user runs or stops services and views logs from the app card; declaration does not mean a service is running.

Use `await agena.fetch(service, path, init)` to access a declared, running service. Use a service-relative path such as `/tasks`; only `Accept` and `Content-Type` request headers are supported. The result is a standard `Response`: check its status before reading `json()` or `text()`, and handle stopped services and failed requests.

Web supports chart interaction and separate app previews. Terminals show text diagrams, app summaries, or source according to their capabilities.
