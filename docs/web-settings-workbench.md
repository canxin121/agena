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

- **Permission Studio** — edit Global, Workspace, current Session, or read-only Effective permission documents. The editor covers filesystem defaults and rules, network zones and domain rules, the default tool policy, tool-name rules, and shell command rules. All rule types support create, rename, mode changes, and delete. Raw `PermissionConfig` JSON remains available.
- **Persistent rules** — inspect and revoke durable approval rules created by interactive permission decisions.

The source selector shows a compact summary for every loaded layer so the user can see whether a decision comes from Global, Workspace, Session, or the merged Effective policy before editing.

### Plugins & Tools

- **Plugin Workbench** — searchable/filterable plugin list with Overview, Config, Tools, Commands, Views, Controls, Capabilities, Logs, and Diagnostics tabs.
  - Plugin Config materializes JSON Schema defaults, applies localized schema overlays, renders nested objects/arrays/enums/unions, supports array reorder/copy/delete, and falls back to raw JSON for open-ended structures.
  - Saves contain the minimal plugin-owned override rather than a copy of every schema default.
  - Dry-run validation and the saved/draft override diff are available before a runtime-reloading save.
  - Manifest tools can be invoked in the active Session with JSON input while their schemas and permission contracts remain visible.
- **MCP Server** — listener enablement, authentication mode, mixed-auth anonymous access, OAuth client registration, public resource URL, issuer URL, OAuth password, endpoint inspection, and tool exposure.
- **Tool harnesses** — named Browser, Shell, and Editor harnesses with explicit Global/Workspace targets, effective-value copying, rename/delete, raw JSON, browser launch options, shell environment variables, and all typed runtime fields.

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
- Plugin configuration saves the full configured-plugin record at the quoted path `plugins.list."<plugin-id>"`, preserving package and timeout fields while replacing only the plugin-owned minimal `config` override and enabled state.
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

Focused contract and unit tests cover the section hierarchy/deep links, advanced layer editor, model verbosity metadata, plugin schema defaults/overrides/unions, plugin config workflow, permission layer summaries, TUI/Web locale separation, and dynamic Interface searches.
