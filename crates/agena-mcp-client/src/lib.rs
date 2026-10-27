//! # agena-mcp-client
//!
//! MCP client wrapper built on top of the official Rust SDK (`rmcp`).
//!
//! Provides the MCP protocol types ([`protocol`]), connection management
//! ([`manager`]), OAuth login sessions ([`oauth`]), token storage
//! ([`token_store`]), and typed errors ([`McpError`] / [`McpResult`]).

pub mod credentials;
pub mod error;
pub mod management;
pub mod manager;
pub mod oauth;
mod oauth_http;
pub mod protocol;
pub mod token_store;

pub use credentials::credential_store_name;
pub use error::{McpError, McpResult};
pub use manager::{
    ConnectedServer, HttpAuth, McpConnectionManager, McpCredentialConflict, McpCredentialState,
    McpServerAuthMode, McpServerStatus, McpToolPolicy, McpToolRisk, ReconnectPolicy, ServerSpec,
    TokenStore,
};
pub use oauth::{
    McpOAuthRegistration, McpOAuthSessionManager, McpOAuthStart, oauth_credential_binding,
};
pub use oauth_http::McpOAuthNetworkAuthorizer;
pub use protocol::{
    CallToolParams, CallToolResult, ContentBlock, GetPromptParams, GetPromptResult,
    ListPromptsResult, ListResourcesResult, ListToolsResult, PromptArgument, PromptDescriptor,
    PromptMessage, ReadResourceParams, ReadResourceResult, ResourceContents, ResourceDescriptor,
    ToolDescriptor,
};
pub use token_store::{
    FileOAuthCredentialStore, FileTokenStore, MCP_SECRET_NAMESPACE, OAuthCredentialHealth,
    OAuthCredentialState, OAuthExpiryState, TokenStoreError,
};
