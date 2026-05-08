# agena-vscode — Status: archived

This VS Code extension was a thin proof-of-concept around the original
`agena exec` CLI. It has not been kept in sync with the unified v1 API
(see `docs/http-api.md`) and is not built by CI.

## What's missing

- Uses `agena exec` directly via child process; should migrate to
  the JSON-RPC app-server surface exported from `crates/agena-api-server`
  or to the in-process `agena-client` SDK.
- No support for streaming responses.
- No permission UI / approval flow integration.

## Decision

The extension is kept in-tree as a starting point for future work
but is not actively maintained. Pull requests welcome; no internal
schedule for revival.
