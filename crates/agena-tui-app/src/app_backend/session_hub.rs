//! Session-center metadata, using the same cross-workspace lists as the Web
//! sidebar. No transcript or runtime ownership crosses this boundary.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::TuiBackend;
use agena_api::{
    queries::{ListSessionsParams, ListWorkspacesParams, Query, QueryResult, SessionListBucket},
    resource::{SessionResource, WorkspaceResource},
};
use agena_tui_session::session_hub::{HubPageTarget, HubPagination, SessionHubSectionKind};
use anyhow::{Result, bail};

pub(crate) const HUB_PAGE_SIZE: u64 = 20;

#[derive(Debug, Clone, Default)]
pub(crate) struct HubCatalogQuery {
    pub pages: BTreeMap<HubPageTarget, usize>,
    pub expanded: BTreeSet<i64>,
    pub search: String,
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
        let QueryResult::Workspaces(workspaces) = self
            .client()
            .query(Query::ListWorkspaces(ListWorkspacesParams {
                limit: Some(HUB_PAGE_SIZE),
                offset: workspace_page as u64 * HUB_PAGE_SIZE,
                include_session_count: true,
                search: search.clone(),
                ..Default::default()
            }))
            .await?
        else {
            bail!("expected a workspace page");
        };
        let mut catalog = HubCatalog {
            workspaces: workspaces.items,
            ..Default::default()
        };
        catalog.pages.insert(
            HubPageTarget::Workspaces,
            HubPagination {
                page: workspace_page,
                has_more: workspaces.page.has_more,
                total: None,
            },
        );
        // Keep the active workspace directly accessible on the first page.
        if workspace_page == 0
            && search.is_none()
            && !catalog
                .workspaces
                .iter()
                .any(|ws| ws.id == self.workspace_id())
            && let QueryResult::Workspace(workspace) = self
                .client()
                .query(Query::GetWorkspace(
                    agena_api::queries::GetWorkspaceParams {
                        workspace_id: self.workspace_id(),
                    },
                ))
                .await?
        {
            catalog.workspaces.insert(0, workspace);
        }
        let mut requests = Vec::new();
        for workspace in &catalog.workspaces {
            let target = HubPageTarget::Directory(workspace.id);
            catalog.pages.insert(
                target,
                HubPagination {
                    total: workspace
                        .session_stats
                        .map(|stats| stats.total as usize)
                        .or(workspace.session_count.map(|count| count as usize)),
                    ..Default::default()
                },
            );
            if query.expanded.contains(&workspace.id) || search.is_some() {
                requests.push((
                    target,
                    ListSessionsParams {
                        workspace_id: Some(workspace.id),
                        ..Default::default()
                    },
                ));
            }
        }
        if search.is_some() {
            requests.push((
                HubPageTarget::Section(SessionHubSectionKind::Search),
                ListSessionsParams {
                    search,
                    ..Default::default()
                },
            ));
        } else {
            for (kind, bucket) in [
                (SessionHubSectionKind::Pinned, SessionListBucket::Pinned),
                (
                    SessionHubSectionKind::Favorites,
                    SessionListBucket::Favorite,
                ),
                (SessionHubSectionKind::Running, SessionListBucket::Running),
                (
                    SessionHubSectionKind::Attention,
                    SessionListBucket::Attention,
                ),
                (SessionHubSectionKind::Recent, SessionListBucket::Recent),
            ] {
                requests.push((
                    HubPageTarget::Section(kind),
                    ListSessionsParams {
                        bucket: Some(bucket),
                        ..Default::default()
                    },
                ));
            }
        }
        let semaphore = Arc::new(tokio::sync::Semaphore::new(4));
        let mut tasks = tokio::task::JoinSet::new();
        for (target, mut params) in requests {
            let backend = self.clone();
            let semaphore = semaphore.clone();
            let mut index = *query.pages.get(&target).unwrap_or(&0);
            params.limit = Some(HUB_PAGE_SIZE);
            params.offset = index as u64 * HUB_PAGE_SIZE;
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
                    index = (total.saturating_sub(1) / HUB_PAGE_SIZE) as usize;
                    params.offset = index as u64 * HUB_PAGE_SIZE;
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
                sessions.insert(session.id, session);
            }
        }
        catalog.sessions = sessions.into_values().collect();
        catalog.sessions.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(catalog)
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
