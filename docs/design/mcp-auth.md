# MCP server authentication

Agena serves itself as a Model Context Protocol server on two transports:

- **stdio** (`agena mcp-server`): a thin client that forwards tool discovery and
  invocation to the long-lived server over the ordinary server API. The local
  process is trusted; the server API is protected by the UI session/token.
- **Streamable HTTP** (`/mcp`): the public surface used by hosted MCP clients
  such as ChatGPT connectors, reverse proxies, and Secure MCP Tunnel.

This document describes the HTTP surface. There is exactly **one**
authentication policy; nothing about it is selectable at runtime.

## Deployment address

Web and TUI expose a dedicated **MCP Server** settings section with one **Agena
access URL**. Enter the address used to reach Agena, for example
`https://agena.example.com` or `127.0.0.1:3210`. The same address can be set at
startup with `--mcp-server-url` / `AGENA_MCP_SERVER_URL`, or through
`PUT /api/v1/server/mcp` with `{"serverUrl":"https://agena.example.com"}`.

For `https://agena.example.com`, Agena derives:

- MCP resource: `https://agena.example.com/mcp`;
- OAuth issuer: `https://agena.example.com`;
- authorization, token, revocation, registration and discovery endpoints on
  that same origin.

An omitted scheme defaults to HTTPS for remote hosts and HTTP for loopback
hosts. HTTPS IP addresses with an explicit port are supported. Remote HTTP is
rejected; local HTTP is accepted only for localhost and loopback IP addresses.
An ordinary remote deployment therefore needs an HTTPS reverse proxy that
routes `/mcp`, `/oauth/*`, `/register`, and `/.well-known/*` to Agena. A path
prefix is not supported by the embedded authorization server. Clearing the
address (`{"serverUrl":null}`) restores the listener-local identity. Request
Host and forwarding headers never determine these addresses.

## One server password

`AGENA_SERVER_PASSWORD` (or `agena server --password`) configures the single
**Server Password**. Web login, TUI/CLI login and MCP OAuth consent all verify
this same credential. MCP has no independent password setting, generator or
password-management endpoint. A TUI or CLI client reads the same environment
variable when opening its management session.

Without a configured server password, Web/TUI retain their local anonymous
management mode, but MCP readiness is false and OAuth authorization cannot
succeed. Agena reports how to configure the password and never generates one.
Public deployments must configure the server password.

A management session token and an MCP OAuth access token serve different
protocols and cannot substitute for one another. The password is shared; the
session and grant lifecycles remain separate.

The older `publicUrl` / `oauthIssuerUrl` control fields, persisted records and
`--mcp-public-url` / `--mcp-oauth-issuer-url` flags remain compatible with existing
split-origin or Secure MCP Tunnel deployments. A routed tunnel resource still
needs a separately reachable issuer origin. These legacy fields cannot be
combined with `serverUrl`; saving the single address replaces both legacy
identity overrides. Existing tunnel routing paths remain intact until that
explicit change.

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
| Access token | JWT, 1 hour, `iss`/`sub`/`aud`/`resource`/`scope`/`jti`/`generation` |
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
   refuses redirects, bounds time and size, and rejects local hostnames,
   private IP literals, loopback and link-local destinations (SSRF).
   Direct connections pin the permitted DNS addresses, including the
   IPv4 `198.18.0.0/15` and IPv6 `fc00::/18` / `fdfe:dcba:9876::/64`
   Fake-IP answers used by Clash/Mihomo, sing-box, Surge and similar TUN proxies.
   These common pools work automatically and share the same classification as
   web search/fetch and TUI remote images. This exception applies only to public
   hostname DNS answers;
   literal Fake-IP URLs remain blocked. TLS still verifies the original
   hostname, and disallowed DNS answers never enter the connector.
   Standard HTTP(S)/SOCKS environment or OS proxies and `NO_PROXY` are honored.
   A CONNECT/SOCKS remote-DNS proxy resolves the destination itself, so its
   public destination hostname is validated without requiring local DNS to
   return the proxy's real target address. Proxy selection is captured once
   and used for the connection. Client metadata and callback checks remain
   mandatory on either route.
   The document must name the same `client_id`, declare 1–16 valid redirect URIs,
   support `token_endpoint_auth_method=none`, and support the authorization-code
   flow.

### Consent

The authorization page asks for the same **Server Password** as Web/TUI
login. The verifier and failed-login counters are shared: switching between
the login and OAuth endpoints does not bypass the per-client or global rate
limits. The page never displays or returns the password.

A salted Argon2 verifier is persisted as shared server authentication state.
The environment/CLI remains authoritative; the stored verifier cannot enable
login after the configured password has been removed. A restart with the same
password reuses the verifier, preserving the OAuth credential version. A new
password creates a new verifier and invalidates existing OAuth grants. OAuth
control stores only a fingerprint of that salted verifier to detect credential
changes; it has no second password verifier.

The standalone browser pages reuse the Web application's theme tokens and Agena
logo. Their HTML, CSS, fixed script and seven locale catalogs live in
`packages/agena-web/src/standalone/oauth`; the Rust renderer embeds them, so
consent does not depend on the frontend bundle or an external asset service.
The page follows the browser's Agena language and appearance preferences when
available, with browser-language and system-theme defaults. Arabic uses RTL.
The native password form works with JavaScript disabled. Cancel submits a
validated `access_denied` callback with `state` and `iss`, without requiring a
password or issuing an authorization code.

Consent and browser error pages use `no-store`, `no-referrer`, anti-framing and
`nosniff` headers. CSP permits only the hash of the embedded UI script, inline
styles and the embedded favicon, and restricts form destinations to Agena and
the validated callback origin. User-controlled values are HTML-escaped and
never reinterpreted as template markers. Invalid client/callback requests stay
on a local error page for browser navigation; protocol callers retain JSON
errors, and validated-client errors retain their OAuth redirect behavior.

Password changes, deployment identity changes and disabling/re-enabling MCP
advance a persisted token generation, invalidating previously signed JWTs as
well as outstanding codes and refresh tokens. Token issuance, consent and
revocation serialize with control changes. Clearing a refresh-token map alone
would not revoke a self-contained JWT.

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
- a deployment without a server password starts with MCP marked unready;
  configure `AGENA_SERVER_PASSWORD` to enable OAuth authorization;
- legacy MCP-only password hashes are discarded on load and omitted from new
  control records. Previously issued OAuth grants are invalidated. Clients
  must authorize again with the server password;
- rename the old `AGENA_SERVER_UI_PASSWORD` environment variable to
  `AGENA_SERVER_PASSWORD`. `--ui-password` remains a hidden compatibility alias
  for `--password`, including old installed service launch arguments. Reinstall
  the native service definition with `agena server install` to capture the new
  environment variable.

## Test coverage

Tests cover the policy and grant surfaces in `apps/agena/src/server/mcp.rs`
(redirect-URI rules, CIMD client-id rules, SSRF address classification, PKCE,
scope normalization, loopback identity handling, public-URL normalization, and
`securitySchemes` rewriting), single-address derivation, nullable control
updates, authorization-code/refresh exchanges with and without a resource
indicator, refresh replay and family revocation, and JWT invalidation after
reset and runtime restoration. Shared tool policy is covered in
`crates/agena-mcp-server/src/stateless_tool_policy.rs` (tool exposure and access
policy). `scripts/agena/tests/test_mcp_oauth.py` exercises the real HTTP surface
using an isolated data directory: DCR, consent, authorization-code exchange,
refresh rotation/replay, revocation, the shared login password, password
changes/removal, same-password restart continuity, disable/enable,
Host/Origin handling, single-address control, browser error negotiation,
passwordless denial, translated password errors and the HTML script's CSP hash.
The standalone renderer also tests locale completeness, RTL, HTML escaping,
template injection, password-field handling and requested permission display.
Run the integration tests against a freshly built binary:

```sh
cargo build -p agena --bin agena
AGENA_TEST_BINARY="$PWD/target/debug/agena" python3 -m unittest discover \
  -s scripts/agena/tests -p test_mcp_oauth.py -v
```
