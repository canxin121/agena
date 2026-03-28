# Agena migration guide: procwarden policy model

This document describes migrating from fixed sandbox modes to the flexible `procwarden` policy model.

## Dependency

Use git dependency in `Cargo.toml`:

```toml
procwarden = { git = "https://github.com/canxin121/procwarden.git", rev = "<pin-commit>" }
```

## Old -> new mapping

- `DangerFullAccess` -> `SandboxPolicy::new_unsandboxed_policy()`
- `ReadOnly` -> `SandboxPolicy::new_read_only_policy()`
- `WorkspaceWrite` -> `SandboxPolicy::new_workspace_write_policy()`

For custom use cases, prefer `new_custom_policy()`.

## Recommended custom policy template

```rust
use std::path::PathBuf;
use procwarden::{SandboxPathPermission, SandboxPolicy};

let policy = SandboxPolicy::new_custom_policy()
    .with_permissions([
        SandboxPathPermission::read_only(PathBuf::from("/workspace/templates")),
        SandboxPathPermission::read_write(PathBuf::from("/workspace/run")),
    ])
    .with_network_access(false)
    .with_reparse_point_rejection(true)
    .with_allow_unc_paths(false);
```

## Consuming enforcement report

Always log or evaluate:

- `output.enforcement.backend`
- `output.enforcement.read_allowlist_enforced`
- `output.enforcement.write_allowlist_enforced`
- `output.enforcement.degraded_reason_codes`

If policy requires strict enforcement, reject runs with:

- `effective_*_enforcement != Strong`
- non-empty `degraded_reason_codes`

## Operational notes

- Linux gives strongest read/write allowlist behavior.
- macOS uses a virtualization-runner backend (set `PROCWARDEN_MACOS_RUNNER` if non-default path is used).
- Windows uses AppContainer as the only sandbox backend.
- `new_unsandboxed_policy()` should be used only as explicit break-glass mode.
