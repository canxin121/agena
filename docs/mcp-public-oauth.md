# Public Agena MCP with built-in OAuth

This is the simplest supported production layout when Agena runs on a server
with a public domain:

```text
ChatGPT / browser
       |
       | HTTPS :443
       v
Caddy or Nginx (TLS termination)
       |
       | HTTP 127.0.0.1:3210
       v
Agena server
  /mcp
  /.well-known/oauth-protected-resource/...
  /.well-known/oauth-authorization-server
  /oauth/authorize
  /oauth/token
  /oauth/revoke
  /oauth/jwks.json
```

Agena remains bound to loopback. The reverse proxy owns the certificate and is
the only process exposed to the Internet.

## 1. DNS and TLS

Create a DNS record such as:

```text
agena.example.com -> your public server IP
```

Use a publicly trusted TLS certificate. Caddy can provision and renew one
automatically.

The easiest configuration uses one origin for both MCP and OAuth:

```text
MCP resource: https://agena.example.com/mcp
OAuth issuer: https://agena.example.com
```

Usually `AGENA_MCP_OAUTH_ISSUER_URL` should be omitted and Agena derives the
issuer origin from `AGENA_MCP_PUBLIC_URL`. A path-bearing issuer is supported
when the reverse proxy preserves that prefix; Agena then publishes RFC 8414
metadata at the corresponding path-aware well-known URL and roots its OAuth
endpoints beneath that issuer.

## 2. Agena environment

Create a root-readable environment file, for example `/etc/agena/agena.env`:

```bash
AGENA_SERVER_UI_PASSWORD=replace-with-a-long-random-password
AGENA_MCP_PUBLIC_URL=https://agena.example.com/mcp
AGENA_MCP_AUTH_MODE=oauth
```

Generate the password rather than inventing a short one:

```bash
openssl rand -base64 32
```

The three values above are sufficient. These secure defaults are already used:

```bash
AGENA_MCP_ANONYMOUS_ACCESS=none
AGENA_MCP_TOOL_EXPOSURE=read-only
AGENA_MCP_CLIENT_REGISTRATION=cimd-only
```

`AGENA_SERVER_UI_PASSWORD` is required for public OAuth. It protects the Web/TUI
management API and is also the OAuth authorization-page password unless an
MCP-specific password is later set from Web or TUI.

Explicit MCP environment variables are authoritative on every process start.
They override an older persisted Web/TUI selection, so a retained database
cannot silently keep using a previous domain or authentication mode.

For a separate authorization domain, proxy that domain to the same Agena
process and set its public issuer:

```bash
AGENA_MCP_OAUTH_ISSUER_URL=https://auth.example.com
```

## 3. Run Agena privately

Run Agena as a dedicated unprivileged user and bind it only to loopback:

```bash
agena --database-path /var/lib/agena/agena.db server \
  --host 127.0.0.1 \
  --port 3210 \
  --workspace /srv/agena/workspace
```

A minimal systemd unit is:

```ini
[Unit]
Description=Agena server
After=network-online.target
Wants=network-online.target

[Service]
User=agena
Group=agena
EnvironmentFile=/etc/agena/agena.env
ExecStart=/usr/local/bin/agena --database-path /var/lib/agena/agena.db server --host 127.0.0.1 --port 3210 --workspace /srv/agena/workspace
Restart=on-failure
RestartSec=3
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/var/lib/agena /srv/agena/workspace

[Install]
WantedBy=multi-user.target
```

Protect the secrets and state:

```bash
chmod 600 /etc/agena/agena.env
chown agena:agena /var/lib/agena /srv/agena/workspace
```

Back up the Agena database. It contains the durable OAuth signing key, hashed
credentials, registered clients, refresh-token records, and revocations. It
does not store plaintext OAuth passwords or plaintext refresh tokens.

## 4. Reverse proxy

### Caddy

```caddyfile
agena.example.com {
    reverse_proxy 127.0.0.1:3210 {
        header_up Host {host}
        flush_interval -1
    }
}
```

### Nginx

```nginx
server {
    listen 443 ssl http2;
    server_name agena.example.com;

    # Configure ssl_certificate and ssl_certificate_key here.

    location / {
        proxy_pass http://127.0.0.1:3210;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Proto https;
        proxy_set_header Authorization $http_authorization;
        proxy_buffering off;
        proxy_read_timeout 3600s;
    }
}
```

Keep the original `Host` header. Agena does not trust forwarded headers to
construct its OAuth identity; it compares the request host with the explicitly
configured resource and issuer. Do not expose port 3210 through the firewall.
Only ports 80/443 should be public.

The proxy must pass these request headers without filtering them:

```text
Authorization
Accept
Content-Type
MCP-Protocol-Version
Mcp-Method
Mcp-Name
Mcp-Param-*
MCP-Session-Id
```

Do not cache `/mcp`, `/oauth/*`, or `/.well-known/oauth-*` responses. Disable
response buffering for MCP streaming responses.

## 5. Verify the deployment

Check public OAuth discovery:

```bash
curl -fsS https://agena.example.com/.well-known/oauth-protected-resource/mcp
curl -fsS https://agena.example.com/.well-known/oauth-authorization-server
curl -fsS https://agena.example.com/oauth/jwks.json
```

The protected-resource document should identify:

```json
{
  "resource": "https://agena.example.com/mcp",
  "authorization_servers": ["https://agena.example.com"]
}
```

An unauthenticated MCP request should be challenged rather than executed:

```bash
curl -i https://agena.example.com/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  --data '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"1"}}}'
```

Expected result: HTTP `401` with a `WWW-Authenticate: Bearer` challenge whose
`resource_metadata` URL points at the public Agena domain.

To inspect Agena's readiness projection, first obtain a management token:

```bash
TOKEN=$(curl -fsS https://agena.example.com/auth/session \
  -H 'Content-Type: application/json' \
  --data '{"password":"replace-with-a-long-random-password"}' | jq -r .token)

curl -fsS https://agena.example.com/api/v1/server/mcp \
  -H "Authorization: Bearer $TOKEN" | jq
```

`ready` should be `true`, and the displayed resource, issuer, discovery URLs,
and token endpoints must all use the public HTTPS domain.

## 6. Connect ChatGPT

In ChatGPT developer mode, add a remote MCP connector using:

```text
https://agena.example.com/mcp
```

ChatGPT can identify itself through its Client ID Metadata Document, so Agena's
secure `cimd-only` default does not require opening unauthenticated Dynamic
Client Registration. ChatGPT opens the Agena authorization page, the user enters
the configured password, and Agena issues resource-bound access and rotating
refresh tokens.

## 7. Exposure policy

The production default is `read-only`. Agena also removes interactive and
autonomous task tools from the remote connector surface.

Only enable this after reviewing every exposed plugin permission contract:

```bash
AGENA_MCP_TOOL_EXPOSURE=all-non-interactive
```

That mode can expose tools with filesystem writes, shell execution, mutation,
or outbound network access. OAuth proves who is authorized; it does not make a
high-impact tool invocation intrinsically safe.

## Troubleshooting

- **Agena refuses to start with OAuth enabled:** set
  `AGENA_SERVER_UI_PASSWORD` and ensure the public resource is HTTPS.
- **Issuer rejected:** use a canonical HTTPS URL. When the issuer contains a
  path, ensure the reverse proxy routes both that OAuth prefix and the matching
  path-aware RFC 8414 well-known URL to Agena.
- **ChatGPT cannot discover OAuth:** verify both well-known URLs from outside
  the server and ensure the proxy routes `/.well-known/*` and `/oauth/*`.
- **Token works before restart but not after:** keep the same Agena database;
  replacing it rotates the signing key and intentionally invalidates tokens.
- **Wrong domain after redeploy:** set `AGENA_MCP_PUBLIC_URL` explicitly. Agena
  never uses `X-Forwarded-Host` to define token issuer or audience.
