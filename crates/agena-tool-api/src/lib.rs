//! Runtime-owned discovery and invocation protocol exposed to model adapters.
//!
//! This crate has no plugin SDK, plugin host, or execution-registry dependency.
//! Protocol functions are fixed declarations. Discovery consumes a read-only
//! catalog projection; only `tools_call` produces an execution request.

mod catalog;
mod dispatch;
mod registry;

pub use catalog::{CatalogPlugin, CatalogTool, ToolApiCatalog};
pub use dispatch::{ExecutionRequest, ToolApiAction, ToolApiError, ToolApiResponse};
pub use registry::ToolApiRegistry;

#[cfg(test)]
mod tests;
