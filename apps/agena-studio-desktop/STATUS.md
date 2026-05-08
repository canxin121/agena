# agena-studio-desktop

Two desktop shells live under this directory:

- `src-tauri/` — primary Tauri 2 shell, the one we ship.
- `src-tauri-cef/` — experimental CEF (Chromium Embedded Framework) shell
  exploring an alternative renderer with full DevTools parity.

Both are workspace-external (their own `Cargo.lock`), kept here so the
frontend in `packages/agena-studio-web` can be exercised against either.

## Build

```bash
# Tauri (primary)
cd apps/agena-studio-desktop/src-tauri && cargo run

# CEF (experimental)
cd apps/agena-studio-desktop/src-tauri-cef && cargo run
```

CI builds the Tauri shell on Windows; CEF is built locally only.

## Status

- `src-tauri` — actively maintained, used in `agena-studio-release.yml`
- `src-tauri-cef` — experimental, no release artifacts
