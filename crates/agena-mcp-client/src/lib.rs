//! `agena-mcp-client` — MCP client wrapper built on top of the official
//! Rust SDK (`rmcp`).

pub mod error;
pub mod manager;
pub mod oauth;
pub mod protocol;
pub mod token_store;

pub use error::{McpError, McpResult};
pub use manager::{
    ConnectedServer, HttpAuth, McpConnectionManager, McpCredentialMigration, McpCredentialState,
    McpServerAuthMode, McpServerStatus, McpToolPolicy, McpToolRisk, ReconnectPolicy, ServerSpec,
    TokenStore,
};
pub use oauth::McpOAuthLoginSession;
pub use protocol::{
    CallToolParams, CallToolResult, ContentBlock, GetPromptParams, GetPromptResult,
    ListPromptsResult, ListResourcesResult, ListToolsResult, PromptArgument, PromptDescriptor,
    PromptMessage, ReadResourceParams, ReadResourceResult, ResourceContents, ResourceDescriptor,
    ToolDescriptor,
};
pub use token_store::{
    FallbackTokenStore, FileTokenStore, KeyringOAuthCredentialStore, KeyringTokenStore,
    MCP_KEYRING_SERVICE, OAuthCredentialHealth, OAuthCredentialState, OAuthExpiryState,
    TokenStoreError,
};
