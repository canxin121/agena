# MCP server authentication

Agena serves itself as a Model Context Protocol server on two transports:

- **stdio** (`agena mcp-server`): a thin client that forwards tool discovery and
  invocation to the long-lived server over the ordinary server API. The local
  process is trusted; the server API is protected by the UI session/token.
- **Streamable HTTP** (`/mcp`): the public surface used by hosted MCP clients
  such as ChatGPT connectors, reverse proxies, and Secure MCP Tunnel.

This document describes the HTTP surface. There is exactly **one**
authentication policy; nothing about it is selectable at runtime.

## Request pipeline

```
POST /mcp
 ├─ require_mcp_enabled        server disabled → 404 (kill switch)
 ├─ validate_mcp_host_origin   Host/Origin must belong to the deployment identity
 ├─ JSON-RPC metadata          method, batch shape, tool calls with ids
 ├─ discovery methods          initialize, ping, tools/list, resources/list,
 │                             prompts/list, notifications/*  → anonymous
 ├─ tools/call                 per tool:
 │      McpToolAccess::Anonymous → proceed
 │      McpToolAccess::OAuth     → valid bearer? proceed
 │                                 otherwise answer that single call with an
 │                                 in-band authentication challenge
 └─ anything else              not advertised by this tools-only server
```

OAuth endpoints are always mounted: `/.well-known/oauth-protected-resource`
(including the `/mcp` and path-suffixed variants),
`/.well-known/oauth-authorization-server`, `/oauth/authorize`, `/oauth/token`,
`/oauth/revoke`, `/oauth/jwks.json`, `/.well-known/jwks.json`, and `/register`.

## Tool policy

`agena-mcp-server::stateless_tool_policy` owns two decisions shared by both
transports:

- `is_stateless_mcp_tool_exposed`: which runtime tools belong on a surface that
  has no Agena session (provider adapters, session/UI/agent/control plugins and
  interactive tools are hidden).
- `mcp_tool_access`: whether one exposed tool may be called anonymously.

`mcp_tool_access` returns `OAuth` for everything until a tool is listed in
`ANONYMOUS_MCP_TOOL_ALLOWLIST`. Read-only is deliberately *not* enough: on a
stateless surface `fs.read` still exposes private workspace data. A tool owner
opts in by adding its name, which switches that tool's advertised
`securitySchemes` entry from `oauth2` to `noauth` and lets anonymous callers
invoke it.

The HTTP adapter rewrites `tools/list` responses at the boundary to add the
Apps SDK `securitySchemes` extension, because the standard MCP `Tool` model has
no such field. A tool missing from the authoritative catalog is treated as
protected, never as anonymous.

## Authorization server

Agena runs a small OAuth 2.1 authorization server next to the resource server:

| Property | Value |
| --- | --- |
| Signing | Ed25519 (`EdDSA`), one persisted key, `kid` = SHA-256 of the public key |
| Access token | JWT, 1 hour, `iss`/`sub`/`aud`/`resource`/`scope`/`jti` |
| Refresh token | opaque 30 days, stored as a SHA-256 fingerprint, rotated on use |
| Replay handling | reuse of a rotated refresh token revokes the whole family |
| Revocation | `POST /oauth/revoke` records the access-token `jti` |
| PKCE | `S256` required, verifier 43–128 chars |
| Scope | `agena:tools` (required), `offline_access` (optional) |
| Token client auth | `none` (public clients only, no client secrets) |
| Client registration | CIMD documents and RFC 7591 dynamic registration |

`resource` (RFC 8707) is optional: a client that omits it receives a grant bound
to this MCP resource; a client that sends it must match exactly. The
`iss` parameter is returned on authorization responses (RFC 9207).

### Client registration

A `client_id` is resolved in two ways:

1. **Dynamic registration** (`POST /register`, RFC 7591). Agena never issues a
   client secret; it validates `redirect_uris` (HTTPS, or loopback HTTP), records
   `client_name`/`grant_types`/`response_types`, and persists the registration
   next to the OAuth runtime state. Records are capped at 256 and the oldest one
   is evicted.
2. **Client ID Metadata Document** (CIMD): the `client_id` is an HTTPS URL that
   hosts the client's own metadata. Any HTTPS origin is accepted, but the fetch
   refuses redirects, bounds time and size, and rejects hosts that resolve into
   loopback, link-local, private, or otherwise non-public address space (SSRF).
   The document must name the same `client_id`, declare 1–16 valid redirect URIs,
   support `token_endpoint_auth_method=none`, and support the authorization-code
   flow.

### Consent

The authorization page asks for the operator password. Agena uses the MCP-specific
password when one is configured, otherwise the server UI password. When neither
exists, the server **generates** a dedicated password on startup, stores it as an
Argon2 PHC hash, logs it once, and reports it once through the control plane so
the deployment is never left with an authorization server nobody can complete.
Failed attempts are rate limited per client and globally.

## Transport hardening

- `Host` must match the deployment identity (configured public URL, configured
  issuer, or a loopback authority). Request headers never define OAuth identity.
- `Origin` is accepted for `chatgpt.com`, `chat.openai.com`, the server's own
  origins, and — when the server only exposes a loopback identity — loopback
  origins. An opaque `null` origin is accepted only for the consent form POST
  that carries `Sec-Fetch-{Site,Mode,Dest}` and a form content type.
- Access tokens are verified in-process: algorithm and `kid` pinned, issuer,
  audience and `resource` compared, `nbf`/`exp` enforced with 5s leeway, scope
  checked, `jti` checked against the revocation list.
- The token endpoint refuses to issue tokens for a non-HTTPS public resource;
  loopback HTTP is allowed for local development only.
- Disabling the MCP server removes the entire public surface (MCP plus OAuth)
  and keeps the management API reachable.

## Migration from selectable modes

Releases before this policy exposed `--mcp-auth-mode`
(`none`/`oauth`/`mixed`), `--mcp-anonymous-access` (`none`/`read_only`), the
matching `AGENA_MCP_*` environment variables, control-plane fields, and TUI/Web
selectors. They are gone:

- persisted control records and older control-plane clients that still send the
  fields keep working — the value is reported once (log / debug trace) and
  ignored, and the unified policy applies;
- a deployment that ran with `--mcp-auth-mode none` now answers unauthenticated
  tool calls with an authentication challenge. Tool *discovery* stays anonymous,
  so connectors still enumerate the catalog;
- a deployment that had no operator password no longer fails to start; Agena
  generates one and prints it once.

## Test coverage

Unit tests cover the pure policy surfaces in `apps/agena/src/server/mcp.rs`
(redirect-URI rules, CIMD client-id rules, SSRF address classification, PKCE,
scope normalization, loopback identity handling, public-URL normalization, and
`securitySchemes` rewriting) and in
`crates/agena-mcp-server/src/stateless_tool_policy.rs` (tool exposure and access
policy). End-to-end coverage of a full authorization-code round trip against a
running server is still missing.
