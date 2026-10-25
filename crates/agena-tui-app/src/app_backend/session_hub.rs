//! Session-center metadata, using the same cross-workspace lists as the Web
//! sidebar. No transcript or runtime ownership crosses this boundary.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::TuiBackend;
use agena_api::{
    queries::{ListSessionsParams, ListWorkspacesParams, Query, QueryResult, SessionListBucket},
    resource::{SessionResource, WorkspaceResource},
};
use agena_tui_session::session_hub::{
    HubPageTarget, HubPagination, HubView, SessionHubSectionKind,
};
use anyhow::{Result, bail};

pub(crate) const HUB_PAGE_SIZE: u64 = 20;

#[derive(Debug, Clone, Default)]
pub(crate) struct HubCatalogQuery {
    pub view: HubView,
    pub revealed_workspace: Option<i64>,
    pub page_size: Option<usize>,
    pub pages: BTreeMap<HubPageTarget, usize>,
    pub expanded: BTreeSet<i64>,
    pub search: String,
}

impl HubCatalogQuery {
    fn session_page_size(&self) -> u64 {
        if self.view == HubView::CurrentProject {
            self.page_size
                .unwrap_or(HUB_PAGE_SIZE as usize)
                .clamp(1, agena_api::pagination::MAX_LIMIT as usize) as u64
        } else {
            HUB_PAGE_SIZE
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct HubCatalog {
    pub workspaces: Vec<WorkspaceResource>,
    pub sessions: Vec<SessionResource>,
    pub pages: BTreeMap<HubPageTarget, HubPagination>,
    pub members: BTreeMap<HubPageTarget, BTreeSet<i64>>,
}

impl TuiBackend {
    pub(crate) async fn session_hub_catalog(&self, query: HubCatalogQuery) -> Result<HubCatalog> {
        let workspace_page = *query.pages.get(&HubPageTarget::Workspaces).unwrap_or(&0);
        let search = (!query.search.trim().is_empty()).then(|| query.search.trim().to_owned());
        let mut catalog = HubCatalog::default();
        match query.view {
            HubView::CurrentProject => {
                catalog
                    .workspaces
                    .push(self.hub_workspace(self.workspace_id()).await?);
            }
            HubView::Projects if search.is_none() => {
                let QueryResult::Workspaces(workspaces) = self
                    .client()
                    .query_revalidated(Query::ListWorkspaces(ListWorkspacesParams {
                        limit: Some(HUB_PAGE_SIZE),
                        offset: workspace_page as u64 * HUB_PAGE_SIZE,
                        include_session_count: true,
                        ..Default::default()
                    }))
                    .await?
                else {
                    bail!("expected a workspace page");
                };
                catalog.workspaces = workspaces.items;
                catalog.pages.insert(
                    HubPageTarget::Workspaces,
                    HubPagination {
                        page: workspace_page,
                        has_more: workspaces.page.has_more,
                        total: None,
                    },
                );
                // Keep the launch project accessible on the first page.
                if workspace_page == 0
                    && !catalog
                        .workspaces
                        .iter()
                        .any(|ws| ws.id == self.workspace_id())
                {
                    catalog
                        .workspaces
                        .insert(0, self.hub_workspace(self.workspace_id()).await?);
                }
                // Adding an existing project may resolve a path outside this
                // page. Keep that explicitly requested project reachable.
                if let Some(id) = query.revealed_workspace
                    && !catalog
                        .workspaces
                        .iter()
                        .any(|workspace| workspace.id == id)
                {
                    catalog.workspaces.push(self.hub_workspace(id).await?);
                }
            }
            HubView::Projects | HubView::AllSessions => {}
        }
        for workspace in &catalog.workspaces {
            catalog.pages.insert(
                HubPageTarget::Directory(workspace.id),
                HubPagination {
                    total: workspace
                        .session_stats
                        .map(|stats| stats.total as usize)
                        .or(workspace.session_count.map(|count| count as usize)),
                    ..Default::default()
                },
            );
        }
        let requests = session_requests(&query, &catalog.workspaces, self.workspace_id());
        let semaphore = Arc::new(tokio::sync::Semaphore::new(4));
        let mut tasks = tokio::task::JoinSet::new();
        let page_size = query.session_page_size();
        for (target, mut params) in requests {
            let backend = self.clone();
            let semaphore = semaphore.clone();
            let mut index = *query.pages.get(&target).unwrap_or(&0);
            params.limit = Some(page_size);
            params.offset = (index as u64).saturating_mul(page_size);
            params.include_total = true;
            params.exclude_subagents = true;
            tasks.spawn(async move {
                let _permit = semaphore.acquire_owned().await?;
                let mut page = backend.list_sessions(params.clone()).await?;
                // Deleting the last item on a page must not strand navigation.
                if index > 0
                    && let Some(total) = page.total
                    && params.offset >= total
                {
                    index = (total.saturating_sub(1) / page_size) as usize;
                    params.offset = (index as u64).saturating_mul(page_size);
                    page = backend.list_sessions(params).await?;
                }
                Ok::<_, anyhow::Error>((target, index, page))
            });
        }
        let mut sessions = BTreeMap::new();
        while let Some(result) = tasks.join_next().await {
            let (target, index, page) = result??;
            catalog.pages.insert(
                target,
                HubPagination {
                    page: index,
                    has_more: page.page.has_more,
                    total: page.total.map(|count| count as usize),
                },
            );
            catalog.members.insert(
                target,
                page.items.iter().map(|session| session.id).collect(),
            );
            for session in page.items {
                // Independently fetched buckets can finish out of order.
                // Never replace a newer resource with an older duplicate.
                let existing: Option<&SessionResource> = sessions.get(&session.id);
                if existing.is_none_or(|old| {
                    (session.version, session.updated_at) >= (old.version, old.updated_at)
                }) {
                    sessions.insert(session.id, session);
                }
            }
        }
        catalog.sessions = sessions.into_values().collect();
        catalog.sessions.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        // Project search uses one global, bounded session page, then groups
        // those results by project. It must find titles in projects outside the
        // first workspace page without adding a second flat result list.
        if query.view == HubView::Projects && search.is_some() {
            let ids = catalog
                .sessions
                .iter()
                .map(|session| session.workspace_id)
                .collect::<BTreeSet<_>>();
            let mut workspaces = tokio::task::JoinSet::new();
            for id in ids {
                let backend = self.clone();
                let semaphore = semaphore.clone();
                workspaces.spawn(async move {
                    let _permit = semaphore.acquire_owned().await?;
                    backend.hub_workspace(id).await
                });
            }
            while let Some(result) = workspaces.join_next().await {
                let workspace = result??;
                catalog.members.insert(
                    HubPageTarget::Directory(workspace.id),
                    catalog
                        .sessions
                        .iter()
                        .filter(|session| session.workspace_id == workspace.id)
                        .map(|session| session.id)
                        .collect(),
                );
                catalog.workspaces.push(workspace);
            }
            catalog.members.remove(&HubPageTarget::Workspaces);
        }
        Ok(catalog)
    }

    async fn hub_workspace(&self, workspace_id: i64) -> Result<WorkspaceResource> {
        let QueryResult::Workspace(workspace) = self
            .client()
            .query_revalidated(Query::GetWorkspace(
                agena_api::queries::GetWorkspaceParams { workspace_id },
            ))
            .await?
        else {
            bail!("expected a workspace");
        };
        Ok(workspace)
    }

    pub(crate) async fn add_hub_project(&self, path: String) -> Result<WorkspaceResource> {
        let result = self
            .client()
            .command(agena_api::commands::Command::ResolveWorkspace(
                agena_api::commands::ResolveWorkspaceParams {
                    path,
                    create_if_missing: true,
                },
            ))
            .await?;
        let agena_api::commands::CommandResult::Workspace(workspace) = result else {
            bail!("expected a workspace while adding a project");
        };
        Ok(workspace)
    }

    pub(crate) async fn create_workspace_session(
        &self,
        workspace_id: i64,
        title: String,
    ) -> Result<SessionResource> {
        Ok(self
            .client()
            .create_session(workspace_id, title, None)
            .await?)
    }

    pub(crate) async fn set_session_pinned(
        &self,
        session_id: i64,
        pinned: bool,
    ) -> Result<SessionResource> {
        Ok(self.client().set_session_pinned(session_id, pinned).await?)
    }
}

/// Plan session reads from the selected scope, independently of rendering.
fn session_requests(
    query: &HubCatalogQuery,
    workspaces: &[WorkspaceResource],
    current: i64,
) -> Vec<(HubPageTarget, ListSessionsParams)> {
    let search = (!query.search.trim().is_empty()).then(|| query.search.trim().to_owned());
    if query.view == HubView::AllSessions {
        return [
            (
                SessionHubSectionKind::Pinned,
                Some(SessionListBucket::Pinned),
            ),
            (
                SessionHubSectionKind::Favorites,
                Some(SessionListBucket::Favorite),
            ),
            (SessionHubSectionKind::All, None),
        ]
        .into_iter()
        .map(|(kind, bucket)| {
            (
                HubPageTarget::Section(kind),
                ListSessionsParams {
                    bucket,
                    search: search.clone(),
                    ..Default::default()
                },
            )
        })
        .collect();
    }
    if query.view == HubView::Projects && search.is_some() {
        return vec![(
            HubPageTarget::Workspaces,
            ListSessionsParams {
                search,
                ..Default::default()
            },
        )];
    }
    let mut requests = Vec::new();
    for workspace in workspaces {
        if query.view == HubView::CurrentProject && workspace.id != current {
            continue;
        }
        if query.expanded.contains(&workspace.id) || search.is_some() {
            requests.push((
                HubPageTarget::Directory(workspace.id),
                ListSessionsParams {
                    workspace_id: Some(workspace.id),
                    search: search.clone(),
                    ..Default::default()
                },
            ));
            if search.is_none() {
                for (kind, bucket) in [
                    (SessionHubSectionKind::Pinned, SessionListBucket::Pinned),
                    (
                        SessionHubSectionKind::Favorites,
                        SessionListBucket::Favorite,
                    ),
                ] {
                    requests.push((
                        HubPageTarget::DirectorySection(workspace.id, kind),
                        ListSessionsParams {
                            workspace_id: Some(workspace.id),
                            bucket: Some(bucket),
                            ..Default::default()
                        },
                    ));
                }
            }
        }
    }
    requests
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_current_project_uses_terminal_sized_pages() {
        let mut query = HubCatalogQuery::default();
        assert_eq!(query.session_page_size(), 20);
        query.page_size = Some(47);
        assert_eq!(query.session_page_size(), 47);
        query.page_size = Some(0);
        assert_eq!(query.session_page_size(), 1);
        query.page_size = Some(usize::MAX);
        assert_eq!(query.session_page_size(), agena_api::pagination::MAX_LIMIT);
        for view in [HubView::Projects, HubView::AllSessions] {
            query.view = view;
            query.page_size = Some(47);
            assert_eq!(query.session_page_size(), 20);
        }
    }

    fn workspaces() -> Vec<WorkspaceResource> {
        [1, 2, 3]
            .into_iter()
            .map(|id| WorkspaceResource {
                id,
                path: format!("/projects/{id}"),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                session_count: None,
                session_stats: None,
            })
            .collect()
    }

    #[test]
    fn current_project_reads_never_escape_its_workspace() {
        for search in ["", " session-title "] {
            let query = HubCatalogQuery {
                expanded: BTreeSet::from([1, 2, 3]),
                search: search.to_owned(),
                ..Default::default()
            };
            let requests = session_requests(&query, &workspaces(), 2);
            assert!(!requests.is_empty());
            assert!(
                requests
                    .iter()
                    .all(|(_, params)| params.workspace_id == Some(2))
            );
            if !search.is_empty() {
                assert_eq!(requests.len(), 1);
                assert_eq!(requests[0].1.search.as_deref(), Some("session-title"));
            }
        }
    }

    #[test]
    fn project_tree_requests_only_expanded_project_buckets() {
        let query = HubCatalogQuery {
            view: HubView::Projects,
            expanded: BTreeSet::from([2]),
            ..Default::default()
        };
        let requests = session_requests(&query, &workspaces(), 1);
        assert_eq!(requests.len(), 3);
        assert!(
            requests
                .iter()
                .all(|(_, params)| params.workspace_id == Some(2))
        );
        assert!(requests.iter().all(|(target, _)| matches!(
            target,
            HubPageTarget::Directory(_) | HubPageTarget::DirectorySection(_, _)
        )));
        assert_eq!(
            requests
                .iter()
                .filter(|(_, params)| params.bucket == Some(SessionListBucket::Pinned))
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|(_, params)| params.bucket == Some(SessionListBucket::Favorite))
                .count(),
            1
        );
    }

    #[test]
    fn all_sessions_reads_global_marks_and_all_sessions_on_independent_pages() {
        for search in ["", "needle"] {
            let query = HubCatalogQuery {
                view: HubView::AllSessions,
                search: search.to_owned(),
                ..Default::default()
            };
            let requests = session_requests(&query, &workspaces(), 1);
            assert_eq!(requests.len(), 3);
            for ((target, params), (kind, bucket)) in requests.iter().zip([
                (
                    SessionHubSectionKind::Pinned,
                    Some(SessionListBucket::Pinned),
                ),
                (
                    SessionHubSectionKind::Favorites,
                    Some(SessionListBucket::Favorite),
                ),
                (SessionHubSectionKind::All, None),
            ]) {
                assert_eq!(*target, HubPageTarget::Section(kind));
                assert_eq!(params.bucket, bucket);
                assert!(params.workspace_id.is_none());
                assert_eq!(
                    params.search.as_deref(),
                    (!search.is_empty()).then_some(search)
                );
            }
        }
    }

    #[test]
    fn grouped_project_search_keeps_one_unbucketed_global_page() {
        let query = HubCatalogQuery {
            view: HubView::Projects,
            search: "needle".to_owned(),
            ..Default::default()
        };
        let requests = session_requests(&query, &workspaces(), 1);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, HubPageTarget::Workspaces);
        assert!(requests[0].1.workspace_id.is_none());
        assert!(requests[0].1.bucket.is_none());
        assert_eq!(requests[0].1.search.as_deref(), Some("needle"));
    }
}
