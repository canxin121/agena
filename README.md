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

Engineering guides:

- [Development validation](docs/development.md)
- [Git workflow and file recovery](docs/git-workflow.md)

## Install

macOS and Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.ps1 | iex
```

To build and install from a checkout instead of a release archive:

```bash
cd /path/to/agena
bash scripts/agena/install.sh install --source
```

The current checkout is used directly. Pass `--source PATH` to build another
checkout or `--source GIT_URL` to clone one; see the
[installation guide](docs/installation.md#install-from-source) for requirements
and Windows usage. To rebuild an existing installation or change its saved bind
address, port, workspace, or data root, run `upgrade` with the new options:

```bash
bash scripts/agena/install.sh upgrade --source --host 0.0.0.0 --port 8080
```

GitHub Releases remain beta releases. The repository and binaries use version
`0.1.0`; beta tags use `agena-v0.1.0-beta.N`. The installer discovers the
newest beta release, verifies its SHA-256, installs the native binary and Web
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
- One instance data directory: `--data-dir DIR` / `AGENA_DATA_DIR` (default `~/agena`), containing the shared SQLite database, configuration, app-managed encrypted secrets, workspace files and client state.
