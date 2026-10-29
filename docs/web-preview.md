# Web preview

Web preview runs a workspace application behind an Agena-owned gateway. The
workbench obtains a short-lived URL for an isolated origin; the application does
not run on the workbench's origin and does not receive its authentication token.
The managed browser can open an owned preview by ID without granting general
access to localhost or private networks.

## Components and request flow

| Component                                          | Responsibility                                                                                     |
| -------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `apps/agena/src/server/preview/registry`           | Persist preview configuration, ownership, and observed process state.                              |
| `apps/agena/src/server/preview/runtime.rs`         | Own child processes and their cleanup; reconcile persisted state after restart.                    |
| `apps/agena/src/server/preview/apps.rs`            | Run multi-service apps, wait for HTTP readiness, cancel pending starts, and maintain widget state. |
| `apps/agena/src/server/preview/access.rs`          | Issue expiring, run-bound preview origins and revoke their grants.                                 |
| `apps/agena/src/server/preview/gateway.rs`         | Dispatch preview hosts before the control router; proxy HTTP and WebSocket traffic.                |
| `apps/agena/src/server/preview/browser.rs`         | Resolve session-owned preview IDs into live managed-browser leases.                                |
| `crates/agena-web/src/managed_preview.rs`          | Share the provider and revocable lease interface with the browser plugin.                          |
| `packages/agena-web/src/features/workspacePreview` | Edit launch settings, request access, renew grants, and render the isolated frame.                 |

The workbench sends an authenticated
`POST /api/v1/workbench/preview/sessions/{id}/access` with `backendOrigin` and
`workbenchOrigin`. The response contains `url`, `expiresAt` (Unix milliseconds),
and `generation`. The frame and the open-in-new-tab action use this URL. They do
not attach the control API bearer token.

A URL has the form `http://p-<random>.localhost:<gateway-port>/` locally, or
`https://p-<random>.preview.example.com/` with an explicitly configured remote
origin. The random hostname is a bearer capability: anyone who has the URL can
use it until it expires or is revoked. Avoid publishing it in public logs or
sharing it as a permanent deployment URL.

The gateway recognizes preview hosts before normal control authentication and
static UI routing. An unknown, expired, or revoked preview host is rejected; it
cannot fall through to the control API or workbench. Requests to the control
origin whose `Origin` identifies a preview, or an opaque `null` origin, are also
rejected. Main API authentication remains separate.

Both the isolated gateway and the legacy proxy reject an upstream targeting the
actual Agena control listening port. That boundary uses the server's configured
listener, not the client-supplied `Host` header. Upstream URLs containing userinfo
(a username or password) are rejected as well.

## Local development and remote deployment

### Local development

For backend origins on `localhost`, `127.0.0.1`, or `::1`, no preview-origin
configuration is required. Agena issues a random `.localhost` hostname using the
gateway's port. The client's browser must resolve that hostname to the machine
or local forwarding endpoint that reaches Agena.

When the workbench runs through Vite, its UI port may differ from the backend
port. The access endpoint derives the gateway authority from the request's
`Host`, rather than assuming the SPA origin is the backend. A Vite proxy that
rewrites `Host` to its backend target therefore produces a preview URL for the
backend port. A local tunnel that preserves its client-facing `Host` can retain
the forwarded port. Check the returned URL when using a custom proxy chain.

The preview URL must reach the Agena gateway, not Vite's workbench server or the
application's upstream development port. HTTPS workbenches also need a preview
origin that the browser can load without mixed-content blocking.

### Remote deployment

Configure an origin when starting the Agena server, for example:

```bash
export AGENA_PREVIEW_ORIGIN=https://preview.example.com
```

The value is an HTTP(S) origin with a DNS hostname. Do not include `*`, a path,
credentials, a query, or a fragment. Agena adds a random subdomain for each grant.
The configuration is read at server startup.

The deployment must provide all of the following:

1. Wildcard DNS for `*.preview.example.com` pointing to the preview ingress.
2. TLS for `*.preview.example.com` when using HTTPS. A certificate covering only
   `*.example.com` does not cover this additional subdomain level.
3. Routing from those hosts to the same Agena server that issued the grants.
   Grants live in memory, so independent replicas cannot validate each other's
   grants without an additional routing or shared-state design.
4. The original preview `Host` preserved when forwarding to Agena. Replacing it
   with the backend's internal hostname loses the capability routing identity.
5. Root paths, query strings, HTTP streaming, and WebSocket upgrade traffic
   forwarded without adding a control API path prefix. Set the forwarded scheme
   correctly when TLS terminates at the ingress.

Use a preview hostname distinct from the workbench hostname. A workbench at
`agena.example.com` and preview base at `preview.example.com` are separate
origins while remaining on the same site, which is useful for normal iframe
cookie behavior. The preview ingress must permit the gateway's own grant-based
access flow; an extra control-token challenge at that ingress cannot be
satisfied by a plain iframe navigation.

A non-local backend without `AGENA_PREVIEW_ORIGIN` returns a configuration
error. It does not fall back to serving executable app documents on the
workbench origin. This setting does not provision DNS, certificates, or reverse
proxy routes; those remain deployment work.

## Grant lifetime and run identity

Grants last 30 minutes. While its preview panel and document are visible, the
workbench renews access every 10 minutes, or earlier if the expiry requires it.
Renewal for the same run, origin, and access policy reuses the hostname, so it
does not reset the iframe's route or application state. Visibility changes
refresh access, stale requests are aborted, and responses from an older
selection cannot replace the current preview.

The grant generation incorporates the preview ID, its server-generated registration
instance ID, workspace and run directory,
owner session, upstream URL, command, arguments, PID, start time, and state.
Control operations revoke access, and each gateway request also compares the
grant with the current registry record. Stopping or restarting a run invalidates
its previous access. Recreating a deleted ID creates a new registration identity,
so a late authorization request cannot revive access to the replacement. Existing
records without an instance ID remain readable. Server restart loses all in-memory grants.

HTTP and WebSocket connections check grant validity and run ownership at admission
and at most one second between checks while waiting for response headers or
streaming. Revocation drops an upstream request or response body; already delivered
bytes cannot be recalled. HTTP response bodies have no fixed overall timeout, so
SSE remains usable. Uploads instead have a 15-second inactivity timeout, followed
by a 15-second response-header wait once the upload finishes. Slow uploads that
continue making progress are not cut off by a total upload deadline.

Mutations capture the grants they are replacing, so a late request for an old run
cannot revoke a newly issued run's grant. A browser tool lease also expires: reopen the preview
when the tool reports an expired or changed run.

## Application HTTP behavior

The isolated gateway proxies the application at `/`. It streams response bodies
and does not rewrite the application's HTML or CSS. Root-relative asset and API
paths therefore stay on the preview origin. Same-upstream redirects within the
configured upstream base path are mapped back to the preview origin, resolving
relative `Location` values against the actual requested resource and removing
that base path once. Redirects outside that mapping retain their original
destination.

Application `Authorization`, `Cookie`, and `Set-Cookie` headers are preserved.
The gateway strips a `Set-Cookie` `Domain` attribute so the cookie is scoped to
the individual preview host, while retaining legitimate cookies named `domain`.
Cookie paths under the configured upstream mount are mapped to the preview root
(`Path=/mount` becomes `Path=/`; `/mount/account` becomes `/account`). It preserves
attributes such as `Secure` and `SameSite`; it does not change an application's cookie policy to bypass
browser rules. In particular:

- A `Secure` cookie still needs an appropriate secure context.
- Browser restrictions on cross-site iframe cookies still apply. A preview URL
  is sufficient for gateway authorization, but it does not guarantee that an
  application's own third-party-cookie login flow will work.
- A restarted run gets a different origin, so its previous origin's cookies and
  local storage are not transferred to the new run.
- Hard-coded upstream hosts, absolute WebSocket endpoints, cookie paths, or
  application-specific external login callbacks may require app configuration.

Hop-by-hop headers, including headers named by `Connection`, are removed. An
`Origin` matching the preview is translated to the upstream origin; foreign
origins are preserved so the application can apply its own CSRF checks.
WebSocket subprotocols and application credentials are forwarded through the
isolated gateway. An ingress still needs to support upgrades and long-lived
connections for HMR.

Upstream Content Security Policy directives are retained except for
`frame-ancestors`, which is replaced with the allowed workbench origin and self.
The gateway removes `X-Frame-Options` for embedding and sets
`Referrer-Policy: no-referrer`. The workbench frame permits scripts, forms,
same-origin behavior within the separate preview origin, and downloads. Its
parent cannot directly inspect the app DOM across the origin boundary; browser
automation uses CDP rather than relying on that access.

### Legacy path compatibility

The older control-origin route
`/api/v1/workbench/preview/s/{id}/...` remains available for compatibility and
authenticated diagnostics. It is not the workbench's normal iframe source.
Executable documents returned through that route receive a CSP sandbox without
`allow-same-origin`, producing an opaque document origin. This intentionally
restricts storage and other origin-dependent features on that compatibility
route.

Diagnostics accept only relative control proxy paths and never follow redirects
with the control bearer token. A manual redirect counts as an upstream readiness
response. The dock retries temporary startup failures for at most 18 seconds;
selection changes, hidden panes or tabs, and unmounting cancel that wait. The legacy HTML/CSS rewriting path has a bounded buffer;
new isolated previews use streaming proxy responses instead.

## Managed browser access

`browser_list` includes `previews` belonging to the current canonical workspace
and Agena session, even if Chrome has not been started. To open one, supply its
ID instead of a URL:

```json
{ "preview_id": "<id from browser_list>" }
```

`browser_open` accepts exactly one of `url` and `preview_id`. A public `url` keeps
the existing public-network policy. A `preview_id` resolves only when the
preview belongs to that workspace and session and the current Agena server owns
its live child process. A persisted PID, a stopped record, or a manually attached
local service does not grant agent browser access. Authenticated users may still
attach a service for human preview through the workbench.

The preview's actual upstream still requires the applicable Agena network
permission. A preview ID provides a narrow exception to the browser's general
public-IP restriction; it does not override the user's Allow, Ask, Auto, or Deny
rules for that upstream. Permission approval and the preview ownership lease
must both succeed.

The target's lease is installed before CDP attachment. Browser actions and
requests validate it, document navigation stays on its exact origin, and stop,
restart, deletion, or expiry invalidates the old target's authority. Another
preview hostname, another port, the control origin, and arbitrary loopback or
private hosts are not covered by this exception.

Agent previews receive a separate hostname from human previews, with an
additional response CSP:

| Directive     | Managed-preview behavior                                                                                                                                     |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `connect-src` | Same-origin HTTP connections and the exact corresponding `ws:` or `wss:` preview origin. External fetch, EventSource, and WebSocket connections are blocked. |
| `frame-src`   | Same-origin child frames only.                                                                                                                               |
| `worker-src`  | Same-origin workers and `blob:` workers.                                                                                                                     |
| `sandbox`     | Scripts, forms, same-origin behavior, and downloads are allowed. Popups and top-level navigation privileges are not granted.                                 |

Other HTTP resources still pass the browser's public-network permission checks.
This CSP is an additional restriction; it does not relax an application's own
CSP. It also covers connections for which CDP Fetch interception alone is not
a sufficient boundary, including WebSocket handshakes. Do not interpret a
policy-blocked external request as proof of an application defect. An app that
requires a third-party API, external frame, popup login, or an external HMR
endpoint may behave differently in a managed browser preview. The human preview
does not receive this additional agent-only policy.

## Process ownership, readiness, and launch arguments

The runtime holds `ManagedChild` handles. Only a currently owned handle can be
stopped; a PID read from the database is not authority to signal an unrelated
process. Startup reconciles stale running records, natural exits update
observed state, and server shutdown closes start admission and cleans up owned
children. Cleanup failures retain ownership for a later retry.

Both interactive Ctrl-C and deployment SIGTERM trigger graceful shutdown on Unix.
Shutdown closes admission and marks all owned processes for cleanup before any
database wait. Process cleanup runs independently of a stalled metadata write.
An owned process that has stopped cannot be silently treated as a human attachment
merely because its PID was cleared; only a never-launched record can represent
an existing external development server. Start rejects an already occupied target
port before launching or truncating logs. This check does not replace network
permissions or prove operating-system ownership of a later listener.

Lifecycle operations serialize per preview or app, rather than holding one
global lock across every readiness wait. Widget state updates use a separate
short-lived lock. App Stop signals cancellation before waiting for its lifecycle
lock, including cancellation of Run requests that were already queued. A
dropped start or readiness request retains cleanup responsibility for the exact
process generation; rollback does not kill a replacement run or a service that
was already running before the failed operation.

Preview configuration writes, app manifest/state writes, and their mutation locks
continue to completion after a requesting HTTP client disconnects. Their locks
cover the actual commit and registry invalidation, preventing a late database
write from overwriting a newer operation or losing its revision notification.
Run remains cancellable. App Stop marks every owned service before awaiting
individual cleanup, and Run rechecks all service generations before reporting
success. Log paths resolve against the same run directory for writes and reads;
non-regular log targets are rejected without blocking on a FIFO.

Multi-service app readiness accepts HTTP 2xx and 3xx responses. A listening port
or a 4xx/5xx response is insufficient. Readiness waits up to 15 seconds and checks
that the process remains owned and running. A failed or cancelled startup rolls
back the services started by that operation and leaves logs available for
diagnosis.

The human preview's readiness probe has a different purpose: it establishes that
the application can answer. The authenticated legacy proxy marks actual upstream
responses with `x-agena-preview-upstream: 1`, allowing a login challenge or an app
error page to render in the isolated iframe. Control authentication and gateway
connection failures do not carry that marker. Health probes cancel when a run,
selection, or visibility changes, and retry startup failures while visible.
Manual refresh is scoped to its pane; a service mutation refreshes only that
service's viewers. An interactive app's Stop can interrupt a pending Run.

The launch form accepts either a JSON array of strings or quoted arguments:

```text
["--name", "My app", "--optional", ""]
--name "My app" --optional ""
```

Arguments are passed directly to process execution. Whitespace inside an
argument, empty arguments, and a zero-argument command are preserved. Quoting
does not enable shell expansion, command substitution, or pipelines. Use a JSON
array when an exact argument vector is easier to express than quoting.

## Verification

From the repository root, run the targeted Rust regressions:

```bash
cargo test -p agena --offline server::preview::
cargo test -p agena-web --offline managed_preview
cargo test -p agena-bundled-plugins --offline managed_preview
cargo test -p agena-runtime-session --lib --offline operator_
```

From `packages/agena-web`, run:

```bash
bun run test:preview
bun run typecheck
```

From the repository root, build a fresh server and run the real Chromium probe:

```bash
cargo build -p agena --bin agena --offline
cd packages/agena-web
AGENA_BIN=../../target/debug/agena bun run test:preview:browser
```

The probe uses Node 22 or newer and Chrome (`CHROME_BIN` may override its path).
It creates a separate temporary server, workspace, data root and browser
profile. It checks password authentication, parent-origin isolation, application
cookies and authorization, root assets, nested redirects, Vite-style WebSocket
messages, and grant revocation without touching the active workbench instance.
It also exercises cookie mount paths, raw encoded request paths, partial/encoded
legacy responses, HTTP/SSE revocation, pending-header cancellation, rejection of
stopped managed targets, and SIGTERM cleanup with a preview still running.
It also invokes the actual operator browser tools to verify owner/workspace
discovery, `browser_open` by preview ID, snapshots, clicks, CSP enforcement, and
denial of actions after a run stops. The probe enables browser tools and allows
its loopback target only in its temporary `data/agena.json`; it does not change
production permissions. `AGENA_PROBE_AGENT=0` skips that tool integration when
only the gateway is being investigated.
Set `PROBE_REPORT` to retain the machine-readable result. This verifies the
gateway's browser behavior; its small fixture UI does not replace Vue tests.

The operator tests exercise plugin-owned file permission callbacks through a
real PluginHost, including authority revocation and failure to elevate Ask or
Auto decisions when no session approval channel exists.

For a deployed gateway, also verify browser-visible behavior against its actual
DNS, TLS, proxy, authentication, and Chrome configuration: an isolated frame,
root-relative assets and nested redirects, app cookies, SSE, HMR/WebSocket
upgrades, stop/restart revocation, and managed-browser rejection of other private
targets. Unit tests alone do not establish that the remote ingress or a specific
Chrome deployment has passed those checks. Record the commands and results for
the environment being tested rather than assuming this document certifies a
deployment.
