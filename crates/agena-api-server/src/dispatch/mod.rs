//! Command/query dispatch through application and runtime service ports.
//! The WS and IPC transports funnel through these helpers so semantics stay
//! identical regardless of transport without naming a concrete session manager.

use std::future::Future;

use agena_api::{
    commands::{
        CancelRunParams, Command, CommandResult, CompactSessionParams, ContinueRunParams,
        CreateSessionParams, CreateWorkspaceParams, DeletePermissionRuleParams,
        DeleteSessionParams, DeleteWorkspaceParams, DismissActivityParams, ExportSessionParams,
        ForkSessionParams, ImportSessionParams, ListSessionTreeParams,
        MarkInteractiveRequestPresentedParams, ReplacePermissionRuleParams, ReplyPermissionParams,
        ReplyUserInputParams, ResolveWorkspaceParams, RevokePermissionRuleParams,
        RewindSessionParams, StopActivityParams, SubmitRunParams, UpdateSessionParams,
        UpdateSessionSelectionParams, UpdateWorkspaceParams, UpsertPermissionRuleParams,
    },
    queries::{
        ActivityLogsParams, GetActivityParams, GetOperationDetailParams, GetPermissionRuleParams,
        GetSessionParams, GetWorkspaceParams, ListPermissionRulesParams,
        ListProviderAdapterModelsParams, ListProviderModelsParams,
        ListSavedProviderAdapterModelsParams, ListSessionsParams, ListWorkspacesParams, Query,
        QueryResult,
    },
    resource::OperationDetailResource,
};
use agena_application::{
    Application, ApplicationError,
    dto::{
        CursorPaginationQuery, PermissionRuleWriteRequest, SearchPaginationQuery, SessionListQuery,
        WorkspaceListQuery, WorkspacePathRequest, WorkspaceResolveRequest,
    },
    pagination::PaginatedResponse as ApplicationPaginatedResponse,
};

async fn http_page_result<T>(
    future: impl Future<Output = Result<ApplicationPaginatedResponse<T>, ApplicationError>>,
) -> Result<agena_api::pagination::PaginatedResponse<T>, ApplicationError> {
    future
        .await
        .map(|page| agena_application::pagination::api_page_from_application(page, |item| item))
}

async fn http_optional_result<T>(
    future: impl Future<Output = Result<Option<T>, ApplicationError>>,
    not_found: impl FnOnce() -> String,
) -> Result<T, ApplicationError> {
    future.await?.ok_or_else(|| {
        ApplicationError::not_found_with_diagnostic("The resource was not found.", not_found())
    })
}

mod commands;
mod queries;

pub use commands::dispatch_command;
pub use queries::dispatch_query;
