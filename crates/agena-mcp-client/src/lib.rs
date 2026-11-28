//! # agena-mcp-client
//!
//! MCP client wrapper built on top of the official Rust SDK (`rmcp`).
//!
//! Provides the MCP protocol types ([`protocol`]), connection management
//! ([`manager`]), OAuth login sessions ([`oauth`]), token storage
//! ([`token_store`]), and typed errors ([`McpError`] / [`McpResult`]).

pub mod credentials;
pub mod error;
mod header_helper;
mod http_transport;
mod legacy_app;
mod legacy_sse;
pub mod apps;
pub mod management;
pub mod manager;
pub mod oauth;
mod oauth_http;
pub mod protocol;
pub mod token_store;
mod websocket;

pub use credentials::credential_store_name;
pub use error::{McpError, McpResult};
pub use header_helper::McpHeaderHelper;
mod redaction;
pub use http_transport::McpHttpNetworkAuthorizer;
pub use manager::{ClaudeChannelEvent, ClaudeChannelPeer, ClaudeChannelStream};
pub use manager::{
    ConnectedServer, HttpAuth, McpConnectionManager, McpConnectionObservation, McpManagedConnection,
    McpCredentialConflict, McpCredentialState, McpElicitationHandler, McpHttpTransport,
    McpProbeOptions, McpProbeResult, McpSandboxContext, McpServerAuthMode, McpServerStatus,
    McpToolPolicy, McpToolRisk, ReconnectPolicy, ScopedPluginServer, ScopedPluginServerResolver,
    ScopedPluginTimeouts, ServerSpec, StdioResourceDeclarations, TokenStore,
};
pub use manager::{
    MCP_TOOL_LOADER, McpOnDemandConfig, McpOnDemandScope, McpOnDemandSelection, McpOnDemandServer,
};
pub use manager::{McpSkill, McpSkillPage, McpSkillResources};
pub use oauth::{
    McpOAuthRegistration, McpOAuthSessionManager, McpOAuthStart, oauth_credential_binding,
};
pub use protocol::{
    CallToolParams, CallToolResult, ContentBlock, GetPromptParams, GetPromptResult,
    ListPromptsResult, ListResourcesResult, ListToolsResult, PromptArgument, PromptDescriptor,
    PromptMessage, ReadResourceParams, ReadResourceResult, ResourceContents, ResourceDescriptor,
    ToolDescriptor,
};
pub use redaction::McpRedactions;
pub use token_store::{
    FileOAuthCredentialStore, FileTokenStore, MCP_SECRET_NAMESPACE, OAuthCredentialHealth,
    OAuthCredentialState, OAuthExpiryState, TokenStoreError,
};
