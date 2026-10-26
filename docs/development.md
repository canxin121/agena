# Development guide

Agena is a Rust workspace with one long-lived HTTP server, a ratatui terminal
client, a Vue Web client, bundled plugins, and packaging scripts. The server
owns sessions, provider calls, tools, configuration, and persistence. Clients
talk to it over HTTP.

This guide describes the current development loop and verification gates. It
does not replace crate-level API documentation.

## Repository map

| Path | Purpose |
| --- | --- |
| `apps/agena` | Main `agena` executable: CLI, server bootstrap, TUI launch |
| `crates/agena-api`, `crates/agena-api-server` | HTTP/JSON API types and server composition |
| `crates/agena-runtime*` | Session runtime, providers, tools, plugins, notifications |
| `crates/agena-storage*` | Storage traits, SQLite implementation, data-root layout |
| `crates/agena-tui*` | Terminal UI application, components, transcript, settings, studios |
| `crates/agena-plugin-*`, `crates/agena-mcp-*` | Plugin and MCP contracts and hosts |
| `crates/agena-bundled-plugins` | Bundled tools and generated tool documentation |
| `packages/agena-web` | Vue 3 Web client, Vite build, icon generator |
| `scripts/agena` | Release installers, source packaging, target matrix |
| `scripts/ci` | Build and capability checks used by CI |
| `scripts/refactors` | Repository refactor checks and their Python tests |
| `tools/agena-web-types` | Generates the Web client's Rust-derived state types |
| `examples` | Example plugins and SDK integrations |
| `third_party` | Vendored patched dependencies; read each `AGENA_PATCH.md` before changing |

The workspace default member is only `apps/agena`. This keeps the common edit
loop focused on the executable. Validation that touches other crates must use
`--workspace`; a bare `cargo clippy` does not cover the whole repository.

## Toolchain

| Tool | Required version |
| --- | --- |
| Rust | `1.97.0`, pinned by `rust-toolchain.toml` |
| Cargo | Installed with Rust; edition 2024 workspace |
| Bun | `1.3.14`, pinned by `.bun-version` |
| Python | 3.10 or newer for repository tooling; CI uses 3.13 |
| Git | Required for source installs and normal repository work |
| C/C++ toolchain | Required by Rust dependencies and platform linking |

Install the pinned Rust components with `rustup`:

```sh
rustup toolchain install 1.97.0 --profile minimal --component rustfmt,clippy
rustup show
```

Use the pinned Bun release:

```sh
bun --version
# 1.3.14
```

Focused Rust tests cover automatic session naming, title persistence, and
auxiliary-model configuration (see [session titles](session-titles.md#verification)).
Focused frontend tests cover browser notification delivery and recovery. The
quality gate compiles all targets, runs lint and contract checks, and builds
the frontend. The Python refactor tooling and
installer contracts still have their own tests. Vendored packages under
`third_party/` retain upstream tests, but those tests are not part of this
repository's quality gate. Run focused first-party tests for the behavior you
change in addition to the compilation and contract checks below.

## Build and run

Build the executable:

```sh
cargo build -p agena --bin agena
```

For an optimized local executable, use `cargo build --release -p agena --bin
agena --locked`. The release profile keeps optimization level 3, enables
incremental compilation for workspace crates, uses 256 codegen units, and
disables whole-program LTO. Dependencies use 16 codegen units and remain
non-incremental so compiler caches such as sccache can reuse them. The first
build after changing a profile rebuilds its affected artifacts; later edits
can reuse the new cache.

Use `cargo build --profile dist -p agena --bin agena --locked` when optimizing
the shipped artifact matters more than rebuild time. That profile uses fat
LTO, one codegen unit, and no incremental compilation. Both optimized profiles
preserve panic unwinding for plugin isolation.

Cargo's crate cache and Rust's incremental cache are different. A fresh crate
can be skipped in either profile, but only incremental compilation can reuse
codegen within a changed workspace crate. sccache does not cache final binary
linking and does not replace that incremental cache.

Unix source installs and packaging use the same native Cargo graph as a plain
`cargo build --release`: `target/release/`, or the target directory selected by
Cargo configuration / `CARGO_TARGET_DIR`. They preserve inherited Rust flags
and read Cargo's reported executable instead of guessing an artifact path.
To build the executable before a source reinstall, run:

```sh
cargo build --release -p agena --bin agena --locked
```

Cross compilation, build-std and extra target-specific Rust flags still use
an explicit `--target` and a separate `target/<triple>/release/` graph. Set
`AGENA_FORCE_EXPLICIT_TARGET=1` to select that graph for a host build too,
for example when overriding a different `build.target` in Cargo configuration.

Windows packaging deliberately uses the shorter `.agena-target/` root to keep
compiler command lines within the Windows process limit. Pass that same
`--target-dir` when reusing its artifacts. Changing the toolchain, target,
features, profile overrides, or `RUSTFLAGS` can invalidate existing artifacts.

To diagnose a slow rebuild, add `--timings` to the actual build command and
inspect the report under the target directory's `cargo-timings/`. Compare an
unchanged second build with a build after editing a workspace source file.
For unexpected recompilation, set
`CARGO_LOG=cargo::core::compiler::fingerprint=info` for one invocation to see
which inputs Cargo considers dirty. Measure the Rust build separately from
the frontend build and archive compression performed by the packaging script.

Start the server in the foreground:

```sh
cargo run -p agena --bin agena -- server
```

The server listens on `127.0.0.1:3210` and uses the `~/agena` data root by
default. When launched directly without `--workspace`, it uses the process
current working directory as the workspace root; the installers pass `$HOME`
as their default workspace. Override the data root per process:

```sh
cargo run -p agena --bin agena -- server \
  --data-dir /tmp/agena-dev \
  --host 127.0.0.1 \
  --port 3210 \
  --workspace /path/to/project
```

In another terminal, start the TUI:

```sh
cargo run -p agena --bin agena -- tui
```

`agena` without a subcommand also starts the TUI. The client resolves the
server URL in this order:

1. `--server` / `AGENA_SERVER_URL`.
2. The live endpoint in `<data-dir>/run/server.json`.
3. `http://127.0.0.1:3210`.

Connections to loopback IP addresses and `localhost` (including `.localhost`
names) bypass environment and system HTTP proxies. Remote Agena endpoints
continue to honor normal proxy settings. This applies to health checks,
password authentication, API requests, and live HTTP streams.

Most one-shot commands, including `agena exec`, `agena sessions`, and
`agena config resolve`, are thin clients of the long-lived server. Start the
server first. `agena inspect` prints the compiled capability manifest and does
not need a live session.

Useful server flags:

```text
--host, --port          Listen address and port
--workspace             Default workspace root
--ui-dir                Built Web frontend directory
--password              Server password shared by Web/TUI login and MCP OAuth
--data-dir              Instance data root
--mcp-enabled           Enable the HTTP MCP surface
--mcp-auth-mode         none, oauth, or mixed
--mcp-public-url        Public MCP resource URL
--mcp-oauth-issuer-url  Public OAuth issuer URL
```

### Use an isolated instance

Use a separate data root and port for each independent server:

```sh
cargo run -p agena --bin agena -- server \
  --data-dir /tmp/agena-work \
  --port 3211
```

Server identity and the endpoint record are scoped to the data root. Two
instances that share a root can conflict over the database and server record;
two instances that share a port cannot both listen. Use a temporary root when
you want disposable state:

```sh
rm -rf /tmp/agena-work
```

Only remove a directory that you created for development and that does not
contain work you need.

## Web frontend

Install dependencies from the frontend package:

```sh
cd packages/agena-web
bun install --frozen-lockfile
```

Start the Vite development server:

```sh
bun run dev
```

Vite serves the SPA and proxies `/api`, `/auth`, and `/health` to
`http://127.0.0.1:3210`. Run the Rust server in another terminal. If no built
frontend exists, the Rust server logs that it is running in API-only mode; the
Vite server is still the UI you open during development.

Build the production frontend:

```sh
bun run build
```

The production build writes `packages/agena-web/dist/`. When `--ui-dir` is
omitted, the server looks for a frontend in:

1. `<workspace>/packages/agena-web/dist`.
2. `packages/agena-web/dist` relative to the current directory.
3. `web-dist` relative to the current directory.
4. `web-dist` next to the installed executable.

An explicit `--ui-dir` is strict: it must contain `index.html`, otherwise
server startup fails instead of silently serving API-only mode.

For a debug frontend with source maps and no minification:

```sh
bun run build:rust-debug
```

That build writes `packages/agena-web/dist-rust-debug/`; pass it explicitly
with `--ui-dir`.

### Browser notifications

Open **Settings → Interface → Web appearance → Browser notifications** and
enable notifications. The first visit also offers an **Enable notifications**
action. Permission is requested only when you click that action or the setting;
the browser retains control over whether to allow it. Wait for the status to
say **Background push is connected**. **Send a test notification** sends through
the real server/push-service/worker path and waits for the browser's receipt;
it reports a connection error or receipt timeout instead of treating construction
of a notification object as proof of delivery.

Notifications cover completed AI replies, questions (including review requests),
permission requests, execution failures, and interruptions. Clicking a
notification focuses Agena and opens the owning session. The server observes
canonical kernel execution and interaction changes, independently of page SSE.
VAPID-signed, encrypted Web Push wakes `/agena-notifications-sw.js`, which calls
`ServiceWorkerRegistration.showNotification()`. There is no app-shell caching.
This continues when Chrome suspends or discards a tab, or the tab is closed.
Repeated events and retries have stable notification identities.

Subscriptions and a bounded retry outbox survive server restarts in the instance
database. The stable VAPID identity lives in the encrypted secret store. Keep the
data directory and secrets when upgrading. Temporary push-service failures are
retried with backoff for up to 24 hours; HTTP 404/410 retires an expired browser
subscription. Disabling notifications removes the server subscription and
unsubscribes the browser. Opening/focusing the app restores and synchronizes
subscriptions, including their localized messages.

Modern desktop Chrome/Chromium, Edge, Firefox, and Safari use the same standard
Push API and Service Worker path; Android browsers with Web Push do too. Support
is detected from browser capabilities. On iOS/iPadOS 16.4+, add Agena to the
Home Screen and enable notifications from that installed app; an ordinary iOS
browser tab cannot subscribe. The settings show this installation guidance.
Push subscription failures are visible and transient errors retry automatically.

Both the backend binary and the web build must be updated. A reverse proxy must
serve the worker as JavaScript at its original root path and forward
`/api/v1/browser-notifications/*`. The server needs outbound HTTPS access to the
browser's push service (Chrome uses `fcm.googleapis.com`). HTTPS or localhost is
required; ordinary LAN HTTP is not a secure context. Browser and operating-system
notification permissions are both required. On macOS, check **System Settings →
Notifications → Google Chrome**, and Focus / Do Not Disturb if the browser receipt
succeeds but no banner appears. Fully quitting the browser, disconnecting the
device, or OS background restrictions can delay delivery until it is available.
Page notifications remain a fallback when background push is unavailable; the
settings explicitly show that this fallback cannot notify a suspended tab.

Run the focused notification tests from `packages/agena-web`:

```sh
bun run test:notifications
```

Run server regressions from the repository root:

```sh
cargo test -p agena --bin agena server::web_push::tests
```

With Playwright and Chrome/Chromium installed, run `bun run
test:notifications:browser` after building the web app. Set `PLAYWRIGHT_MODULE`
to an external Playwright module path and `CHROME_BIN` to a browser executable
when needed. The probe freezes a real Chromium page, then closes it and stops
its worker, injects push events with CDP, and checks native registered
notifications and worker restart. It does not mock Notification APIs or claim
to verify an operating-system banner or Google's external delivery service.

### Frontend build gates

`bun run build` runs these checks in order:

1. `check:icons` verifies every generated icon against the Shift source.
2. `check:imports` enforces the frontend import boundary rules.
3. `check:i18n-settings` verifies settings translations.
4. `check:i18n-messages` verifies message translations.
5. `check:i18n-locales` verifies the locale set.
6. `typecheck` runs `vue-tsc`.
7. `vite build` creates the production bundle.

Formatting scripts are available as `bun run fmt` and `bun run fmt:check`.
They are not part of the CI quality gate.

## Verification matrix

Run the same checks locally that CI runs. The exact commands are:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --locked -p agena-plugin-host --all-targets --all-features -- -D warnings
python3 scripts/ci/verify-runtime-capabilities.py
git diff --exit-code
```

The second clippy command matters because optional plugin-host features are not
enabled by the default workspace build.

From `packages/agena-web`:

```sh
bun install --frozen-lockfile
bun run build
bun audit
git diff --exit-code
```

The `git diff --exit-code` steps catch generated files or lockfiles that were
not updated after a build.

Repository tooling checks:

```sh
python3 -m unittest discover -s scripts/refactors/tests
python3 scripts/refactors/check-refactor-invariants.py \
  --manifest scripts/refactors/failure-semantics-invariants.json
bash -n scripts/dependencies/check.sh
```

For installer changes, also run:

```sh
bash -n scripts/agena/install.sh
python3 -m unittest discover -s scripts/agena/tests -v
python3 scripts/agena/verify-target-coverage.py
```

### What each Rust gate covers

| Command | What it proves |
| --- | --- |
| `cargo fmt --all -- --check` | Every workspace crate follows the committed formatting configuration |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | All workspace crates and build targets compile without warnings |
| `cargo clippy -p agena-plugin-host --all-features` | Optional plugin-host feature combinations compile |
| `verify-runtime-capabilities.py` | The Bedrock auth dependency graph keeps Rustls/ring and bundled Mozilla roots, and does not silently drop provider or TLS capability |
| `git diff --exit-code` | A clean build did not rewrite tracked sources |

`--locked` makes Cargo fail instead of updating `Cargo.lock` implicitly. Keep
the lockfile change in the same logical commit when a dependency change
intentionally updates it.

### CI workflows

| Workflow | Coverage |
| --- | --- |
| `quality.yml` | Rust format/lint on Linux and macOS, optional plugin features, runtime capabilities, tracked-source stability; Web build, audit, and lockfile stability; Python refactor tests and invariants |
| `installer-ci.yml` | Offline installer contracts and full install/upgrade/rollback/uninstall lifecycles on Linux, macOS, and Windows |
| `agena-release.yml` | Builds the six published targets, packages archives and checksums, publishes installers, and smokes the published installers |
| `ci.yml` | Manual universal-target diagnostic: verifies target coverage and runtime capabilities |
| `dependency-report.yml` | Scheduled dependency updates, audits, and unused-dependency report |

## Generated files

Some tracked files are generated from Rust or from the icon source. Regenerate
them when their inputs change and review the diff before committing.

| Output | Regenerate with |
| --- | --- |
| `crates/agena-bundled-plugins/generated/bundled-capability-identities.json` | `cargo run --locked -p agena-bundled-plugins --example generate_capability_identity_snapshot > crates/agena-bundled-plugins/generated/bundled-capability-identities.json` |
| `crates/agena-bundled-plugins/generated/tools-reference.md` | `cargo run --locked -p agena-bundled-plugins --example generate_tools_reference > crates/agena-bundled-plugins/generated/tools-reference.md` |
| `packages/agena-web/src/generated/agenaState.ts` | `cargo run -p agena-web-types > packages/agena-web/src/generated/agenaState.ts` |
| `packages/agena-web/public/` app icons | `cd packages/agena-web && bun run icons:generate` |

The tool reference can also be printed from a built executable:

```sh
cargo run -p agena --bin agena -- inspect --tools-reference
```

Regenerate tool documentation after changing a bundled tool schema or help
text. Regenerate the Web state mirror after changing session, execution,
background-activity, or notification state types. The generated files are the
reviewable contract between the backend and its clients.

## Icons

The editable app icon source is
`packages/agena-web/src/assets/agena.svg`. The generator in
`packages/agena-web/scripts/generate-app-icons.mjs` produces the browser,
Apple, Android, Safari, and manifest assets. See
[the icon guide](design/agena-icon.md) for the full asset list and design
notes.

After editing the source:

```sh
cd packages/agena-web
bun run icons:generate
bun run check:icons
bun run build
```

`check:icons` fails when a generated asset is missing or differs from the
source. Do not edit files under `public/` by hand.

## Dependencies

The dependency entrypoint has two modes:

```sh
bash scripts/dependencies/check.sh check
bash scripts/dependencies/check.sh report
```

`check` is the strict gate. It verifies the pinned Bun version, runs
`cargo-deny`, `cargo-audit`, `cargo-machete`, and the frontend `bun audit`.
`report` prints advisory, upgrade, and unused-dependency information without
turning every finding into a failure. It additionally requires `cargo-upgrade`.

The required tools are `cargo-deny`, `cargo-audit`, `cargo-machete`, and
`cargo-upgrade` for report mode. Advisory exceptions live in `deny.toml` and
must stay synchronized with the arguments in the script.

The frontend `package.json` contains exact overrides for transitive packages
with security or compatibility constraints. Change them together with the
lockfile and verify the affected renderer, editor, and typography paths. Keep
`bun.lock` committed.

## Data, logs, and diagnostics

The data root is the instance's only local storage location for runtime state.
It contains
the SQLite database, configuration, encrypted secrets, per-workspace data,
caches, logs, plugin data, client state, and `run/server.json`. The full layout
is documented in [the installation guide](installation.md#installation-and-data-layout).

Useful diagnostics:

```sh
# Foreground server: logs go to the terminal.
cargo run -p agena --bin agena -- server

# Installed or detached server status.
agena server status

# Structured diagnostics from the current instance.
agena diagnostics --format json

# TUI logging.
agena tui --log-file /tmp/agena-tui.log
agena tui --log-stderr
```

Set `AGENA_LOG` to control the tracing filter, for example
`AGENA_LOG=debug`. Database and provider-adapter filters have separate
variables, `AGENA_DATABASE_LOG` and `AGENA_ADAPTER_LOG`. Server logs for a
service-managed instance are under `<data-dir>/logs/`.

If a foreground server exits during startup, check the terminal output first,
then the data root's `logs/` and `run/server.json`. A stale endpoint record is
detected by the lifecycle commands, but it must be inspected before manual
removal.

For a clean development instance, point `--data-dir` at a new empty directory.
The server creates the layout on startup. This avoids modifying your normal
`~/agena` session history and credentials.

## Packaging and release builds

Build a release archive with the same layout the installer consumes:

```sh
bash scripts/agena/package-backend.sh
```

The script builds the frontend (unless `AGENA_WEB_DIST_PREBUILT=1`), builds the
`agena` binary in release mode, stages `bin/agena` and `web-dist/`, and writes
the archive and `.sha256` under `artifacts/agena/`. A target can be supplied as
the first argument:

```sh
bash scripts/agena/package-backend.sh aarch64-unknown-linux-gnu
```

Unchanged frontend sources and verified build output reuse the successful
Web build, including its validation gates. Changes to sources, environment,
tool version or built files trigger a complete frontend build. Set
`AGENA_WEB_FORCE_BUILD=1` to run it explicitly. Unix packaging also reuses an
unchanged archive after verifying its full SHA-256 and checksum file; binary,
Web output and packaging recipe changes invalidate that cache.

On Windows:

```powershell
.\scripts\agena\package-backend.ps1 -TargetTriple x86_64-pc-windows-msvc
```

The published target list is in
`scripts/agena/universal-targets.json`. Verify that every advertised target has
installer and packaging support with:

```sh
python3 scripts/agena/verify-target-coverage.py
python3 scripts/agena/target-matrix.py native-backend --compact
python3 scripts/agena/target-matrix.py cross-backend --compact
```

The release workflow builds all six targets, publishes the backend archive,
checksum, `install.sh`, `install.ps1`, and installer checksums, then runs the
published-installer smoke test across Linux, macOS, and Windows.

For a source install without publishing, see
[the installation guide](installation.md#install-from-source). The installer
reuses the current checkout when it finds one, so `install --source` from this
directory does not clone a second copy.

## Architecture documentation

Crate documentation is generated from the Rust sources:

```sh
cargo doc --workspace --no-deps --open
```

Put durable architecture descriptions in crate-level or module-level rustdoc,
where they sit next to the code and can be built with `cargo doc`. The
generated tool reference is embedded into rustdoc from
`crates/agena-bundled-plugins/generated/tools-reference.md`.

The runtime's Git behavior is defined in
`crates/agena-runtime-contracts/src/identity/git_workflow.md` and included in
the system prompt at compile time. The explanation for people is
[the Git workflow guide](git-workflow.md).

## Before committing

1. Review `git status` and the full diff.
2. Regenerate any affected generated artifact.
3. Run the narrow checks for the area you changed.
4. Run the workspace and frontend gates when the change crosses crate or API
   boundaries.
5. Confirm the build left tracked files unchanged.
6. Commit implementation, generated files, lockfile updates, and related
   documentation as one reviewable change.
