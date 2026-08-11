//! Command/query dispatch through application and runtime service ports.
//! The WS and IPC transports funnel through these helpers so semantics stay
//! identical regardless of transport without naming a concrete session manager.

use std::future::Future;

use agena_application::{
    Application, ApplicationError,
    dto::{
        CursorPaginationQuery, ModelCatalogResponse as ApplicationModelCatalogResponse,
        PermissionRuleWriteRequest, SearchPaginationQuery, SessionListQuery, WorkspaceListQuery,
        WorkspacePathRequest, WorkspaceResolveRequest,
    },
    pagination::PaginatedResponse as ApplicationPaginatedResponse,
};
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
    pagination::{PageInfo, PaginatedResponse},
    queries::{
        ActivityLogsParams, GetActivityParams, GetOperationDetailParams, GetPermissionRuleParams,
        GetSessionParams, GetWorkspaceParams, ListActivitiesParams, ListPermissionRulesParams,
        ListProviderAdapterModelsParams, ListProviderModelsParams,
        ListSavedProviderAdapterModelsParams, ListSessionsParams, ListWorkspacesParams, Query,
        QueryResult,
    },
    resource::{ModelCatalogResponse, ModelCatalogSourceKind, OperationDetailResource},
};

const fn model_catalog_source_kind_from_domain(
    value: agena_provider::ModelCatalogSnapshotSourceKind,
) -> ModelCatalogSourceKind {
    match value {
        agena_provider::ModelCatalogSnapshotSourceKind::Generated => {
            ModelCatalogSourceKind::Generated
        }
        agena_provider::ModelCatalogSnapshotSourceKind::Cache => ModelCatalogSourceKind::Cache,
    }
}

trait ApplicationResultExt<T> {
    fn application(self) -> Result<T, ApplicationError>;
}

impl<T> ApplicationResultExt<T> for Result<T, ApplicationError> {
    fn application(self) -> Result<T, ApplicationError> {
        self
    }
}

trait IntoWire<T> {
    fn into_wire(self) -> T;
}

impl<T> IntoWire<T> for T {
    fn into_wire(self) -> T {
        self
    }
}

impl IntoWire<ModelCatalogResponse> for ApplicationModelCatalogResponse {
    fn into_wire(self) -> ModelCatalogResponse {
        let value = self;
        ModelCatalogResponse {
            refreshing: value.refreshing,
            last_refresh_at: value.last_refresh_at,
            last_successful_source: value
                .last_successful_source
                .map(model_catalog_source_kind_from_domain),
            last_failure: value.last_failure,
            model_count: value.model_count,
        }
    }
}

fn page_from_application<T, U>(value: ApplicationPaginatedResponse<T>) -> PaginatedResponse<U>
where
    T: IntoWire<U>,
{
    PaginatedResponse {
        items: value.items.into_iter().map(IntoWire::into_wire).collect(),
        page: PageInfo {
            next_cursor: value.page.next_cursor,
            has_more: value.page.has_more,
            returned: value.page.returned as u64,
        },
    }
}

async fn http_page_result<T, U>(
    future: impl Future<Output = Result<ApplicationPaginatedResponse<T>, ApplicationError>>,
) -> Result<PaginatedResponse<U>, ApplicationError>
where
    T: IntoWire<U>,
{
    Ok(page_from_application(future.await.application()?))
}

async fn http_optional_result<T, U>(
    future: impl Future<Output = Result<Option<T>, ApplicationError>>,
    not_found: impl FnOnce() -> String,
) -> Result<U, ApplicationError>
where
    T: IntoWire<U>,
{
    future
        .await
        .application()?
        .map(IntoWire::into_wire)
        .ok_or_else(|| {
            ApplicationError::not_found_with_diagnostic("The resource was not found.", not_found())
        })
}

mod commands;
mod queries;

pub use commands::dispatch_command;
pub use queries::dispatch_query;
