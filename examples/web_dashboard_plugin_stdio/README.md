# Workspace board — a Web plugin with its own backend

This example ships a complete browser page and a Rust stdio backend in one plugin package. The host loads its ES modules after installation; the Agena Web bundle does not import this example.

The board provides task creation, editing, deletion, status changes, search, progress, English/Chinese copy, responsive layouts and light/dark themes. Three clearly described sample tasks seed a new board. Changes are written to plugin-private global storage. A board revision prevents a stale tab from overwriting a newer edit.

## Files

| File | Role |
| --- | --- |
| `src/main.rs` | Plugin manifest, Method commands, validation and durable storage |
| `web/dist/dashboard.js` | Native page entry with `mount(container, sdk)` |
| `web/dist/dashboard.css` | Scoped page styles |
| `web/dist/model.js` | Data validation, filtering and localized copy |
| `web/dist/icons.js` | Local SVG icons |
| `agena-plugin.toml` | Release metadata and included browser files |

The files in `web/dist` are hand-authored release assets. No Node packages, frontend build step, external fonts or CDN requests are required.

## Build and open

Run from the Agena repository root:

~~~sh
cargo build -p agena-web-dashboard-plugin-stdio
~~~

Merge this entry into the instance configuration's `plugins.list`, replacing both absolute paths:

~~~json
{
  "demo.web_dashboard": {
    "enabled": true,
    "package_root": "/absolute/path/agena/examples/web_dashboard_plugin_stdio",
    "package": {
      "kind": "stdio",
      "command": "/absolute/path/agena/target/debug/agena-web-dashboard-plugin-stdio",
      "args": [],
      "env": {}
    }
  }
}
~~~

The instance configuration is `<data-dir>/agena.json`. Reload the instance configuration, then open **Settings → Plugins & Tools**, select **demo.web_dashboard**, and choose **Open interactive page · Workspace board**, or open:

~~~text
/plugins/demo.web_dashboard/dashboard
~~~

Use a separate development data directory and unused port when testing. For example, write the configuration above under `plugins.list` in `/tmp/agena-web-demo/agena.json`, build the Agena executable and Web frontend, then run:

~~~sh
cargo run -p agena --bin agena -- server \
  --data-dir /tmp/agena-web-demo \
  --host 127.0.0.1 \
  --port 3211 \
  --workspace /absolute/path/agena
~~~

The `package_root` points to the plugin project directory; `surface.web.assets.root` points to `web/dist` inside it. Installing a packaged release records the extracted package root automatically.

## Commands and persistence

`list` accepts `{}` and returns a board in `CommandResult.output`:

~~~json
{
  "schema_version": 1,
  "revision": 1,
  "next_id": 4,
  "tasks": [
    {
      "id": "task-1",
      "title": "Explore your plugin workspace",
      "description": "Sample task",
      "status": "planned",
      "priority": "normal",
      "updated_at_ms": 0
    }
  ]
}
~~~

`update` saves one task. A null `id` creates a task:

~~~json
{
  "revision": 1,
  "operation": {
    "type": "save",
    "id": null,
    "title": "Try the example",
    "description": "Add a task, reload, then check that it remains.",
    "status": "in_progress",
    "priority": "normal"
  }
}
~~~

Deletion uses `{"revision": 2, "operation": {"type": "delete", "id": "task-4"}}`. Successful writes return the new board. Revision conflicts return a failed command with `{"conflict": true, "board": ...}`; the page retains the editor draft and asks the user to review and save again.

The page has instance scope, so these operations do not require a chat session. Storage uses scope `global`, visibility `private`, namespace `board`, and key `state`. The host binds the storage request to the plugin identity.

## Package

From the repository root:

~~~sh
cargo run -p agena-plugin-dev --bin agena-plugin -- package \
  --manifest examples/web_dashboard_plugin_stdio/agena-plugin.toml \
  --artifact target/debug/agena-web-dashboard-plugin-stdio \
  --output /tmp/agena-web-dashboard-release
~~~

The archive includes the executable, README and browser assets. Publish the generated release metadata using the normal plugin release workflow; the frontend does not require a separate deployment.

## Verify

~~~sh
cargo test -p agena-web-dashboard-plugin-stdio
node --test examples/web_dashboard_plugin_stdio/web/tests/model.test.mjs
~~~

The host's Web plugin browser probe exercises the real stdio command and persistence path. Stable page selectors include `[data-plugin-dashboard]`, `[data-task-item]`, `[data-action="new"]`, `[data-action="refresh"]`, `[data-action="save"]` and the editor's named fields.

See [Plugin Web pages](../../docs/plugin-web-pages.md) for the public loading, SDK and lifecycle rules.
