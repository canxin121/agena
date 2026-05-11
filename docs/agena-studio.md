# Agena Studio

`agena-studio` is the Studio product shell for the `agena` runtime.

This repository now uses a clear top-level split:

- `apps/` for runnable Rust applications
- `crates/` for reusable Rust libraries
- `packages/` for frontend packages
- `ops/` for packaging, install, and release automation

## Layout

- `apps/agena-studio-server/`: Axum backend binary named `agena-studio`
- `packages/agena-studio-web/`: Vue frontend for chat, login, and runtime settings
- `apps/agena-studio-desktop/`: Tauri desktop wrappers
- `ops/agena-studio/`: build, packaging, install, and release scripts

## Local Development

Build the frontend:

```bash
bun install --cwd packages/agena-studio-web
bun run --cwd packages/agena-studio-web build
```

Run the server against the built frontend:

```bash
cargo run --manifest-path apps/agena-studio-server/Cargo.toml --bin agena-studio -- \
  --ui-dir packages/agena-studio-web/dist
```

Useful runtime flags:

- `--config <path>`
- `--set <key=value>`
- `--workspace-root <path>`
- `--database-path <path>`
- `--database-url <url>`
- `--ui-password <password>`

## Workspace / CI

`apps/agena-studio-server/` is a member of the top-level `agena` Cargo
workspace, so root workspace checks already cover the Studio backend:

```bash
cargo check --workspace --locked
cargo test --workspace --locked
```

The repository-level workflows that exercise and release `agena-studio` live in:

- `.github/workflows/ci.yml`
- `.github/workflows/agena-studio-release.yml`

## Backend Packaging

Build a redistributable backend archive that bundles the `agena-studio` binary
and the built frontend assets:

```bash
./ops/agena-studio/scripts/package-backend.sh
```

On Windows:

```powershell
./ops/agena-studio/scripts/package-backend.ps1
```

Both variants also accept an explicit target triple, for example
`x86_64-pc-windows-msvc`.

## Desktop Packaging

For a full desktop bundle, use the wrapper scripts that build the web assets,
prepare the sidecar, and invoke Tauri in one step:

```bash
./ops/agena-studio/desktop/build-full.sh
```

On Windows:

```powershell
./ops/agena-studio/desktop/build-full.ps1
```

The experimental CEF variant uses `build-full-cef.sh` /
`build-full-cef.ps1`.

Release-style desktop outputs are written under:

- `artifacts/agena-studio/desktop/<target>/standard`
- `artifacts/agena-studio/desktop/<target>/cef`

The internal Cargo/Tauri build cache is isolated under `artifacts/t/`.

## Install Scripts

The backend release archives can be installed as an Agena-native background
service:

- Unix: `./ops/agena-studio/scripts/install-service.sh --version 0.1.0`
- Windows: `./ops/agena-studio/scripts/install-service.ps1 -Version 0.1.0`

These scripts install the backend archive contents (`bin/agena-studio` and
`web-dist/`) and register a user-level background launcher directly against the
current Agena Studio layout.

## Desktop Sidecar

To prepare the backend sidecar used by Tauri packaging:

```bash
./ops/agena-studio/desktop/prepare-sidecar.sh
```

On Windows:

```powershell
./ops/agena-studio/desktop/prepare-sidecar.ps1
```

Both scripts build `apps/agena-studio-server/` and copy the resulting
`agena-studio` binary into the Tauri `binaries/` directory for the selected
desktop variant.

## Status

The runtime-facing migration is complete for:

- `apps/agena-studio-server`
- `packages/agena-studio-web`
- `apps/agena-studio-desktop/src-tauri`
- `apps/agena-studio-desktop/src-tauri-cef`

Legacy studio-specific docs, scripts, tests, and copied source trees that no
longer apply to Agena Studio were removed instead of being kept as stale
compatibility baggage.
