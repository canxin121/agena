# Agena

A local LLM agent runtime. A single long-lived HTTP server (built on axum) owns
all session and AI interaction, settings, and features. Clients — including the
ratatui TUI — talk to it purely over HTTP.

## Documentation

API documentation is generated from source via rustdoc:

```bash
cargo doc --workspace --no-deps --open
```

Deployment guides:

- [Installation and upgrades](docs/installation.md)
- [Public MCP server with built-in OAuth](docs/mcp-public-oauth.md)

## Install

macOS and Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.ps1 | iex
```

Only beta GitHub Releases are published. The installer discovers the newest
`agena-vX.Y.Z-beta.N` prerelease, verifies its SHA-256, installs the native binary and Web
frontend, and starts Agena as a per-user service. See the
[installation guide](docs/installation.md) for upgrade, stop/start, uninstall,
custom workspace/port, and pinned-version commands.

## Development quick start

First start the HTTP server:

```bash
cargo run -p agena --bin agena -- server
```

Then, in another terminal, start the TUI client:

```bash
cargo run -p agena --bin agena -- tui
```

One-shot CLI commands also go through the server rather than starting their own
runtime. (The exact subcommand names are mid-refactor — if in doubt, just run
the server, then run the TUI.)

## Requirements

- Rust (see `rust-toolchain.toml`)
- SQLite (default database at `~/agena/agena.db`)
