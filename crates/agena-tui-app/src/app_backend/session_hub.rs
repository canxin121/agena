//! Session-center metadata, using the same cross-workspace lists as the Web
//! sidebar. No transcript or runtime ownership crosses this boundary.
use std::collections::{BTreeMap, BTreeSet};

use agena_api::{
    pagination::PageInfo,
    queries::{ListSessionsParams, ListWorkspacesParams, Query, QueryResult},
    resource::{SessionResource, WorkspaceResource},
};
use anyhow::{Result, bail};

use super::TuiBackend;

#[derive(Debug, Clone)]
pub(crate) struct HubCatalog {
    pub workspaces: Vec<WorkspaceResource>,
    pub sessions: Vec<SessionResource>,
}

fn next_page(page: PageInfo, seen: &mut BTreeSet<String>) -> Result<Option<String>> {
    if !page.has_more {
        return Ok(None);
    }
    let Some(cursor) = page.next_cursor.filter(|cursor| !cursor.is_empty()) else {
        bail!("session center pagination is missing its next cursor");
    };
    if !seen.insert(cursor.clone()) {
        bail!("session center pagination did not advance");
    }
    Ok(Some(cursor))
}

impl TuiBackend {
    pub(crate) async fn session_hub_catalog(&self) -> Result<HubCatalog> {
        let directories = async {
            let mut cursor = None;
            let mut seen = BTreeSet::new();
            let mut workspaces = BTreeMap::new();
            loop {
                let result = self
                    .client()
                    .query(Query::ListWorkspaces(ListWorkspacesParams {
                        cursor,
                        limit: Some(200),
                        ..Default::default()
                    }))
                    .await?;
                let QueryResult::Workspaces(page) = result else {
                    bail!("expected a workspace page");
                };
                for item in page.items {
                    workspaces.insert(item.id, item);
                }
                cursor = next_page(page.page, &mut seen)?;
                if cursor.is_none() {
                    break;
                }
            }
            Ok::<_, anyhow::Error>(workspaces.into_values().collect::<Vec<_>>())
        };
        let sessions = async {
            let mut cursor = None;
            let mut seen = BTreeSet::new();
            let mut sessions = BTreeMap::new();
            loop {
                let result = self
                    .client()
                    .query(Query::ListSessions(ListSessionsParams {
                        cursor,
                        limit: Some(200),
                        exclude_subagents: true,
                        ..Default::default()
                    }))
                    .await?;
                let QueryResult::Sessions(page) = result else {
                    bail!("expected a session page");
                };
                for item in page.items {
                    sessions.insert(item.id, item);
                }
                cursor = next_page(page.page, &mut seen)?;
                if cursor.is_none() {
                    break;
                }
            }
            let mut sessions = sessions.into_values().collect::<Vec<_>>();
            sessions.sort_by(|a, b| {
                b.updated_at
                    .cmp(&a.updated_at)
                    .then_with(|| b.id.cmp(&a.id))
            });
            Ok::<_, anyhow::Error>(sessions)
        };
        let (workspaces, sessions) = tokio::try_join!(directories, sessions)?;
        Ok(HubCatalog {
            workspaces,
            sessions,
        })
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
