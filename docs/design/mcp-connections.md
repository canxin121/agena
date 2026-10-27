# External MCP connections

Agena is an MCP **client** when it connects to an external service through its
built-in `agena.mcp` plugin. Manage these connections in **Settings → MCP
connections** in Web or TUI. **MCP Server** is a separate setting for exposing
Agena to another client; it does not configure outbound connections.

## Connection management

The manager supports creating, inspecting, editing, deleting, enabling,
disabling, importing, testing, and reconnecting named connections. Web uses the
existing settings layout, theme tokens, responsive panels, and seven locale
catalogs. TUI offers a searchable list, structured editor, advanced JSON editor,
capability browser, import preview, and keyboard help. Both clients use the same
typed management API and configuration store.

Two persisted scopes are available:

- **Global** writes the running server's global configuration file.
- **Workspace** writes the running server workspace's `.agena/agena.json`.

The page displays the exact server workspace and configuration paths. Selecting
another directory for a chat does not create another MCP runtime or redirect
these settings. Each Agena server process owns one connection manager.

Workspace definitions replace complete global definitions with the same name.
Other global connections remain available. A disabled workspace definition still
shadows the global copy; disabling it never reconnects the global service.
Removing the workspace copy makes the global definition effective again.
Non-MCP plugins retain their existing configuration merge semantics.

Edits and deletions require the revision returned by the latest read. Concurrent
changes fail with a refresh instruction instead of overwriting newer settings.
Successful saves reload the runtime. AI tool mutations queue reload after the
originating tool finishes and return a reload task ID; saving is distinct from
confirmed activation. A persisted definition that differs from the active
runtime is shown as **Pending reload**, including after a failed reload or when
another configuration layer overrides it. Retry the existing runtime reload
control in Diagnostics and inspect profile/command-line overrides if necessary.
Other server processes using the same global file need their own reload.

## Transports and capabilities

Supported transports are **stdio** and **Streamable HTTP**. Stdio command and
arguments remain separate values; shell parsing is not applied. Relative working
directories resolve against the server workspace. Per-connection connect and
request timeouts and tool include/exclude filters are available.

The client negotiates the 2026-07-28 discovery lifecycle with automatic fallback
to the 2025-11-25 initialization lifecycle. Server capabilities determine which
catalogs are queried: resource-only or prompt-only servers do not need a tools
capability. Tools, resources, resource templates, and prompts can be inspected;
resources can be read and prompts expanded with named string arguments. Catalog
pagination and change notifications are supported. The client advertises roots;
sampling and elicitation are not advertised.

Connection status comes from the transport, including a closed child process.
Disconnected tools disappear from the active catalog, and the configured
reconnect supervisor retries failures. **Test connection** creates an isolated
client and closes it afterwards, so disabled or shadowed definitions can be
checked without replacing the active connection.

Standalone legacy SSE-only endpoints are not treated as Streamable HTTP. An
import explicitly declaring an unsupported transport is rejected.

## Authentication and secrets

HTTP connections support no authentication, an inline bearer token, an
environment-variable bearer token, an encrypted stored bearer token, custom
headers, and OAuth. Environment values, header values, inline tokens, OAuth
client secrets, and credential-bearing URLs are redacted in management reads.

A read returns `[REDACTED]` plus JSON pointers in `secret_paths`. Updates submit a
complete configuration and `preserve_secrets` for unchanged hidden fields.
Replacing or removing a field also removes its preservation pointer. A
placeholder without explicit preservation is rejected. The editor distinguishes
keep, replace, and clear; it never silently replaces a secret with the sentinel.
Import previews are redacted and the original import document is held only in
client memory until confirmation.

OAuth supports dynamic client registration, pre-registered client IDs and
optional client secrets, and client metadata URLs where the provider supports
them. Sign-in returns a URL and an expiring flow ID. Web handles a matching return
to its settings route; Web, TUI, and AI also support pasting the complete callback
URL. The server retains PKCE state, verifies the callback and connection
revision, and consumes successful flows once. Cancel discards only the pending
flow. OAuth logout deletes the OAuth record without deleting a stored bearer.

Stored credentials are bound to both connection name and endpoint. OAuth records
also bind scopes and registration settings; changing them requires a new
sign-in. Older name-only records remain untouched but are not sent to a new or
changed endpoint. Existing installations using those records must enter the
bearer token again or sign in once. No credential is returned by credential
management operations.

## Import and AI tools

Import accepts native `{ "servers": { ... } }` definitions or common
`{ "mcpServers": { ... } }` JSON. Common stdio entries use `command`, `args`,
`env`, and `cwd`; HTTP entries use `url` and `headers`, with optional `type` of
`http` or `streamable-http`. Supported native authentication, timeouts, and tool
filters are retained. Unknown or transport-incompatible fields are rejected
rather than silently dropped.

Preview validates each candidate, reports name conflicts, and returns a revision
bound to the source and destination. Select candidates and explicitly allow
replacement when desired. All selected entries are validated before one atomic
configuration write. Limits are 200 connections and 4 MiB per import document.

The `agena.mcp` plugin exposes `connections.list`, `connections.get`,
`connections.create`, `connections.update`, `connections.delete`,
`connections.set_enabled`, `connections.import_preview`, `connections.import`,
`connections.test`, `connections.reconnect`, credential operations, and OAuth
start/complete/cancel/logout. Existing discovery and invocation tools continue
to expose external tools, resources, and prompts. Configuration file access,
subprocess execution, network access, and credential-store writes retain the
originating AI tool's permission context. Authorization precedes side effects;
creating a connection does not implicitly grant execution permission.

The authenticated application endpoints live under `/api/v1/mcp-connections`.
Management remains available when the bridge is disabled, so Web and TUI can
repair configuration and enable it again. AI can also restore a disabled bridge
through the existing settings plugin.
