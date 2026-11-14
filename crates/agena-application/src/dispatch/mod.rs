//! Shared business dispatch for HTTP, IPC and plugin HostClient operations.
//! Transport adapters only decode requests and encode the typed results.

use std::future::Future;

use crate::{
    Application, ApplicationError,
    dto::{
        CursorPaginationQuery, SearchPaginationQuery, SessionListQuery, WorkspaceListQuery,
        WorkspacePathRequest, WorkspaceResolveRequest,
    },
    pagination::PaginatedResponse as ApplicationPaginatedResponse,
};
use agena_api::{
    commands::{
        CancelRunParams, ClearSessionParams, Command, CommandResult, CompactSessionParams,
        ContinueRunParams, CreateSessionParams, CreateWorkspaceParams, DeletePermissionRuleParams,
        DeleteSessionParams, DeleteWorkspaceParams, DismissActivityParams, ForkSessionParams,
        ListSessionTreeParams, MarkInteractiveRequestPresentedParams, ReplacePermissionRuleParams,
        ReplyPermissionParams, ReplyUserInputParams, ResolveWorkspaceParams,
        RevokePermissionRuleParams, RewindSessionParams, SessionClearMode, StopActivityParams,
        SubmitRunParams, UpdateSessionParams, UpdateSessionSelectionParams, UpdateWorkspaceParams,
    },
    queries::{
        ActivityLogsParams, GetActivityParams, GetPermissionRuleParams, GetSessionParams,
        GetWorkspaceParams, ListPermissionRulesParams, ListProviderAdapterModelsParams,
        ListProviderModelsParams, ListSavedProviderAdapterModelsParams, ListSessionsParams,
        ListWorkspacesParams, Query, QueryResult,
    },
};

async fn page_result<T>(
    future: impl Future<Output = Result<ApplicationPaginatedResponse<T>, ApplicationError>>,
) -> Result<agena_api::pagination::PaginatedResponse<T>, ApplicationError> {
    future
        .await
        .map(|page| crate::pagination::api_page_from_application(page, |item| item))
}

async fn optional_result<T>(
    future: impl Future<Output = Result<Option<T>, ApplicationError>>,
    not_found: impl FnOnce() -> String,
) -> Result<T, ApplicationError> {
    future.await?.ok_or_else(|| {
        ApplicationError::not_found_with_diagnostic("The resource was not found.", not_found())
    })
}

mod business;
mod commands;
mod extended_sessions;
mod management;
mod queries;
mod streams;

pub use commands::dispatch_command;
pub use queries::dispatch_query;
