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

- [Public MCP server with built-in OAuth](docs/mcp-public-oauth.md)

## Quick start

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
