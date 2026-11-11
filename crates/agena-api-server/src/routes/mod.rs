//! Per-domain REST routers.
//!
//! Each submodule owns one domain's `.route(...)` chain. [`router`] merges them
//! into the same set of REST routes that used to live inline in `lib.rs`; the
//! request-counting middleware is applied by the caller after the merge so it
//! still runs exactly once for every route.

mod activity;
mod bots;
mod git;
mod health;
mod mcp;
mod memories;
mod ops;
mod permissions;
mod plugins;
mod providers;
mod runtime;
mod sessions;
mod settings;
mod skills;
mod streams;
mod tools;
mod workspaces;

use axum::Router;

use crate::AppState;

/// Merge every per-domain router into the REST surface.
pub fn router() -> Router<AppState> {
    Router::new()
        .merge(ops::routes())
        .merge(health::routes())
        .merge(sessions::routes())
        .merge(plugins::routes())
        .merge(mcp::routes())
        .merge(providers::routes())
        .merge(skills::routes())
        .merge(memories::routes())
        .merge(permissions::routes())
        .merge(settings::routes())
        .merge(workspaces::routes())
        .merge(git::routes())
        .merge(activity::routes())
        .merge(runtime::routes())
        .merge(bots::routes())
        .merge(tools::routes())
        .merge(streams::routes())
}
