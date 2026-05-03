# Agena Plugin System

Agena's plugin system lets you extend the runtime with custom tools, hooks
into chat / permission / shell / event flows, and reverse-call the host. A
single `Plugin` trait powers four transports: in-process, cdylib, stdio
subprocess, and HTTP service. Pick the transport per plugin in config.

This document is a primer; the source of truth is the SDK at
`crates/agena-plugin-sdk` and the host at `crates/agena-plugin-host`.

## Quick start: a cdylib plugin

```toml
# Cargo.toml of your plugin crate
[package]
name = "my-plugin"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
abi_stable = "0.11"
agena-plugin-sdk = { path = "…/agena/crates/agena-plugin-sdk", features = ["cdylib"] }
async-trait = "0.1"
serde_json = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

```rust
// src/lib.rs
use agena_plugin_sdk::prelude::*;

#[derive(Default)]
pub struct MyPlugin;

#[async_trait]
impl Plugin for MyPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("my-plugin", "0.1.0")
            .hooks(HookSubscription::TOOL_INVOKE | HookSubscription::SHELL_ENV)
            .entry(
                PluginEntryDecl::new("greet", json!({
                    "type": "object",
                    "properties": { "name": { "type": "string" } },
                    "required": ["name"]
                }))
                .description("Say hello.")
            )
            .build()
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
        let name = input.input.get("name").and_then(|v| v.as_str()).unwrap_or("world");
        Ok(ToolInvokeOutput::text(format!("hello, {name}!")))
    }

    async fn shell_env(&self, _: ShellEnvInput) -> Result<Option<ShellEnvPatch>> {
        Ok(Some(ShellEnvPatch::set("MY_PLUGIN", "1")))
    }
}

agena_plugin_sdk::export_cdylib!(MyPlugin);
```

Build with `cargo build`. Then in your `agena.toml`:

```toml
[plugins.list.my-plugin]
kind = "cdylib"
path = "/abs/path/to/libmy_plugin.so"
options = { greeting = "hi" }
```

## Choosing a transport

| Transport | When to use | Host overhead | Cross-language |
|---|---|---|---|
| `static` | Compiled into agena | None | No |
| `cdylib` | Rust-only, want zero-IPC | abi_stable FFI | No |
| `stdio` | Any language; long-running subprocess | one tokio task per child | Yes |
| `http` | Remote service; share across many agena instances | network round-trip | Yes |
| `wasm` | Untrusted code; sandbox guarantees from wasmtime | wasm linear memory copy | Yes (any wasm-targeting language) |

The same `Plugin` trait impl works on the first four — you only change the
export macro / config kind. Use `export_cdylib!` for cdylib, `export_stdio!`
for subprocess, `export_http!` for an axum server. WASM plugins implement
the same JSON-above-FFI dispatch contract directly (see "WASM ABI" below).

`wasm` and `signing` features ship behind cargo features (`plugin-wasm`,
`plugin-signing` on the `agena` crate; or `wasm` / `signing` on
`agena-plugin-host` directly). Build with
`cargo build --features agena/plugin-wasm,agena/plugin-signing` to enable
both.

## Available hooks

Every method on `Plugin` has a default no-op so you only override what you
need. Declare the subset you care about in `manifest().hooks` so the host
can skip dispatch.

| Method | When fired | Patch type |
|---|---|---|
| `init(ctx, host)` | Once at load | `InitOutcome` |
| `shutdown()` | Once at unload | `()` |
| `tool_invoke(input)` | Plugin-provided tool called | `ToolInvokeOutput` |
| `tool_invoke_stream(input, sink)` | Streaming tool call (host pulls chunks via `invoke_tool_stream`) | `ToolStreamEnd` |
| `tool_execute_before(input)` | Before *any* tool call | `Option<ToolBeforePatch>` |
| `tool_execute_after(input)` | After *any* tool call | `Option<ToolAfterPatch>` |
| `chat_message(input)` | Outgoing user→provider message | `Option<ChatMessagePatch>` |
| `chat_params(input)` | Provider request params | `Option<ChatParamsPatch>` |
| `chat_headers(input)` | Provider HTTP headers (per-request, per-provider) | `Option<ChatHeadersPatch>` |
| `chat_system_transform(input)` | System prompt assembly | `Option<ChatSystemTransformPatch>` |
| `event(envelope)` | Every domain event on the unified bus | — (notification) |
| `auth(input)` | Provider credential lookup miss | `Option<AuthOutput>` |
| `provider_list(input)` | Provider registry enumeration | `Option<ProviderListPatch>` |
| `permission_ask(input)` | Permission check before fallback to user | `Option<PermissionAskDecision>` |
| `command_execute_before(input)` | Before bash subprocess spawn | `Option<CommandBeforePatch>` (incl. `abort`) |
| `shell_env(input)` | Bash env var injection | `Option<ShellEnvPatch>` |
| `config_resolved(input)` | Config loaded / reloaded | `Option<ConfigPatch>` |
| `session_compacting(input)` | Session compaction strategy | `Option<SessionCompactingPatch>` |

Hooks are dispatched **sequentially** across plugins in deterministic order
(by config key). Patches chain: each plugin sees the previous plugin's
output as its input.

## Calling back into the host

Plugins receive an `Arc<dyn HostClient>` in `init`. Sensitive callbacks are
capability-gated: at least one entry in the plugin manifest must declare the
matching `PluginEntryDecl::host_capability(...)`, otherwise the host rejects
the callback before dispatching it.

Example:

```rust
PluginEntryDecl::new("inspect-config", json!({"type": "object"}))
    .description("Read selected runtime config.")
    .host_capability(HostCapability::ReadConfig)
```

Available callbacks include:

- `log(level, message, fields)` — appears in agena's tracing output as
  `target=plugin`; no capability is required.
- `read_config(path)` — read the resolved config tree (dot-paths supported,
  e.g. `runtime.session_cache.max_sessions`).
- `subscribe_events(filter)` / `unsubscribe_events(id)` — manage event
  subscriptions. Stdio/HTTP plugins must implement the `event` hook; the host
  bridge auto-pushes to plugins that subscribed via `HookSubscription::EVENT`.
- `invoke_tool(tool, input)` — call another plugin's tool. **Reentrancy is
  blocked**: a plugin cannot invoke a tool that maps back to itself in the
  same call stack (returns `cycle detected`).
- `publish_event(env)` — injects a `PluginEvent` envelope into the unified
  domain bus. The synthetic event's payload is opaque JSON; subscribers can
  match on `kind = "plugin_event"` and inspect `plugin_id` / `kind_label`
  inside the payload.
- `ask_user(req)`, `spawn_subtask(req)`, `list_tools()`, `monitor_*(...)`, and
  `skill_get(req)` expose first-party runtime substrate to plugins.
- `ask_permission(req)` — currently returns `Prompt` and is not capability
  gated.
- `execute_builtin_tool(req)` — internal legacy bridge reserved for
  `agena.builtin`; third-party plugins should not call it.

| HostClient method / JSON-RPC method | Required `HostCapability` |
|---|---|
| `read_config` / `host/config.read` | `ReadConfig` |
| `invoke_tool` / `host/tool.invoke` | `InvokeTool` |
| `ask_user` / `host/ask_user` | `AskUser` |
| `spawn_subtask` / `host/subtask.spawn` | `SpawnSubtask` |
| `list_tools` / `host/tool.list` | `ListTools` |
| `publish_event` / `host/event.publish` | `PublishEvent` |
| `subscribe_events`, `unsubscribe_events` / `host/event.*` | `SubscribeEvents` |
| `monitor_start`, `monitor_list`, `monitor_read`, `monitor_stop` / `host/monitor.*` | `MonitorRegistry` |
| `skill_get` / `host/skill.get` | `SkillsManager` |

For stdio / HTTP plugins, callbacks travel back over the same JSON-RPC wire
(stdio multiplexed on stdin/stdout, HTTP via `POST /plugin-rpc/{plugin_id}`
on the agena HTTP API server).

`skill_run` is provided by the first-party static plugin `agena.skills`, not by
the core tool catalog. It uses `skill_get` to read skill content and returns the
same payload metadata shape as the previous built-in adapter, including
`allowed_tools`.

## Config schema

```toml
[plugins]
enabled = true
# Optional global timeout overlay; per-plugin overrides take precedence.
timeouts = { tool_invoke = "60s", permission_ask = "10s" }

# Optional ed25519 trusted public keys (hex), looked up by `key_id` in
# cdylib `signature` fields.
[plugins.trusted_keys]
my-vendor = "9c1f...32-byte-pubkey-in-hex"

[plugins.list.echo]
kind = "cdylib"
path = "/abs/path/to/libecho.so"
options = { uppercase = true }
# Optional supply-chain hardening (requires `signing` cargo feature):
# sha256 = "deadbeef…"
# signature = { key_id = "my-vendor", signature = "abcdef…" }

[plugins.list.lint]
kind = "stdio"
command = "node"
args = ["/path/to/lint/index.js"]
env = { LINT_LEVEL = "warn" }
restart = { policy = "on-failure", min_backoff = "1s", max_backoff = "30s", max_retries = 5 }
options = { project = "rust" }
# sha256 = "…"   # optional, requires `signing`

[plugins.list.cloud-policy]
kind = "http"
url = "https://policy.example.com/agena/rpc"
auth = { kind = "bearer", token_env = "AGENA_POLICY_TOKEN" }
options = { org_id = "acme" }

[plugins.list.fuzz-classifier]
kind = "wasm"
path = "/abs/path/to/classifier.wasm"
options = {}
# sha256 = "…"   # optional, refuses to load on mismatch
```

Timeout strings accept `ms`, `s`, `m`, `h` units. Restart policies:
`never`, `on-failure`, `always`. Auth kinds: `none`, `bearer`, `basic`.

## Tool naming

When two plugins (or a plugin + a built-in) declare the same tool name,
the host auto-prefixes both as `<plugin_id>__<tool>`. Built-ins always win
the un-prefixed name. Plugins can opt into a permanent prefix via
`PluginEntryDecl::expose_as("…")`.

## Failure isolation

- Each hook call is wrapped in a per-hook timeout (configurable). On
  timeout the plugin is skipped for that call; the chain continues.
- Cdylib calls run inside `catch_unwind`; a panicking plugin returns
  `Panicked` and is skipped.
- Stdio plugins are supervised: on child exit the host respawns according
  to `RestartPolicy`. In-flight requests fail with `Disconnected`.
- HTTP failures are isolated to the request; transient errors are
  surfaced as `Disconnected`.
- A single plugin's failures never propagate beyond its own hook chain.

## Lifecycle

1. `RuntimeSnapshot::build` reads `[plugins.list]`, spawns each transport,
   sends `meta/init`, parses `InitOutcome`, registers tools.
2. `config` hook is fired with the resolved config.
3. The runtime-backed `HostClient` is installed into the plugin host's
   `HostHandle` so plugin → host callbacks work.
4. The unified `EventBus` is bridged to the plugin host; events flow to
   subscribed plugins.
5. On runtime drop or reload, `meta/shutdown` is sent and transports are
   closed (cdylib: shutdown fn pointer; stdio: SIGKILL after grace; http:
   client drop). The event bridge task is aborted.

## Streaming tool output

For tools that emit incremental output (LLM streaming, long shell tail,
progress bars), implement `tool_invoke_stream(input, sink)` instead of (or
in addition to) `tool_invoke`. The `ToolStreamSink::text(...)` /
`ToolStreamSink::chunk(...)` methods push frames to the host as they're
produced; the final `ToolStreamEnd` is the aggregate equivalent of a
non-streaming response. The host consumes via
`PluginHost::invoke_tool_stream(handle, input).await`, which returns a
`ToolInvokeStream { chunks, end, stream_id }`. Plugins that don't override
`tool_invoke_stream` get a single-chunk emulation for free.

## Hot-reload

When agena reloads its config (file watcher or explicit
`runtime/reload` API), the new snapshot inspects the previous snapshot's
plugin entries. **Byte-identical entries reuse the existing transport** —
the subprocess / HTTP plugin keeps running across the reload. Entries that
changed are torn down (`meta/shutdown`, transport close) and the new
versions are spawned. Entries that disappear are simply shut down.

This means you can add, remove, or retune most plugins by editing the
config file; agena restarts only what actually changed.

## WASM ABI (advanced)

WASM plugins do not use the SDK's `Plugin` trait — they live below the
JSON-RPC layer and only need to expose two exports:

```text
(func (export "agena_alloc") (param i32) (result i32))
(func (export "agena_dispatch")
      (param i32 i32 i32 i32) (result i64))
(memory (export "memory") 1)
```

`agena_alloc(n)` returns a pointer to `n` writable bytes in the module's
linear memory; `agena_dispatch(method_ptr, method_len, params_ptr,
params_len)` returns a packed `i64`: high 32 bits = result pointer, low 32
bits = result length. Set the high bit of the length (i.e. `len |= 0x8000_0000`)
to signal an error; the host then deserialises the bytes as `PluginError`.

A return length of `0` means `Value::Null` — useful for hooks whose
"no opinion" answer is `null`.

The wasm transport is sandboxed: no host imports are exposed in this
release, so wasm plugins cannot call back into agena. Use them for
self-contained classifiers, validators, or scoring functions.

## Plugin signing

When the `signing` (or `agena/plugin-signing`) cargo feature is enabled:

- `cdylib` and `wasm` entries can carry a `sha256 = "<hex>"` field; the
  host hashes the file at load time and refuses to proceed on mismatch.
- `cdylib` entries can carry a `signature = { key_id, signature }` field;
  the host looks up `key_id` in `[plugins.trusted_keys]` and verifies the
  ed25519 signature over the cdylib bytes.
- `stdio` entries can carry a `sha256 = "<hex>"` field; the host hashes
  `command` (when it's a path that exists) and refuses to launch on
  mismatch.

Without the feature, any of these fields cause a load-time error so config
mistakes don't silently degrade to "unsigned ok".

## Reference

- SDK: `crates/agena-plugin-sdk/src/`
- Host: `crates/agena-plugin-host/src/`
- Examples: `examples/echo_plugin/` (cdylib) and
  `examples/echo_plugin_stdio/` (stdio bin)
- Integration tests: `crates/agena-plugin-host/tests/`
- Runtime wiring: `crates/agena/src/runtime/{host_client,event_bridge,snapshot,builder}.rs`
- Config types: `crates/agena-plugin-host/src/config.rs`
