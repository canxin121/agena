## Desktop Packaging (Tauri)

English | [简体中文](README.zh-CN.md)

This folder contains the Tauri desktop wrappers for `agena-studio`.

Current variants:

- `src-tauri`: standard WebView2/Wry build
- `src-tauri-cef`: experimental CEF build

The desktop app talks to a bundled `agena-studio` backend sidecar instead of an
external proxy target.

## Local Quickstart

```bash
bun install --cwd ../../packages/agena-studio-web
bun run --cwd ../../packages/agena-studio-web build
../../ops/agena-studio/desktop/prepare-sidecar.sh
cargo check --manifest-path src-tauri/Cargo.toml
```

For the CEF variant:

```bash
../../ops/agena-studio/desktop/prepare-sidecar.sh --cef
cargo check --manifest-path src-tauri-cef/Cargo.toml
```
