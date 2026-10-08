//! Resource access shared by every transport and every kind of client.

use agena_domain::{ContentCursor, ContentId, ContentPage, ContentResource};
use agena_storage::content::ContentHub;
use agena_storage::store::{Part, SessionChange, Subscription};
use tokio::sync::watch;

use crate::{Application, ApplicationError};

pub struct ContentWatch {
    pub resource: ContentResource,
    pub changes: Option<watch::Receiver<ContentResource>>,
    pub access: watch::Receiver<bool>,
    hub: ContentHub,
    _membership: Subscription,
}

/// One snapshot/subscribe barrier and cursor loop for SSE, WS and IPC.
/// A slow transport retains one cursor and rereads bounded resource pages.
pub struct ContentDelivery {
    watch: ContentWatch,
    cursor: Option<ContentCursor>,
    max_bytes: usize,
    waiting: bool,
    closed: bool,
}

impl ContentDelivery {
    pub async fn next(&mut self) -> Result<Option<ContentPage>, ApplicationError> {
        if self.closed {
            return Ok(None);
        }
        if self.waiting {
            if let Some(changes) = self.watch.changes.as_mut() {
                tokio::select! {
                    result = changes.changed() => {
                        if result.is_err() { self.watch.changes = None; }
                    },
                    _ = self.watch.access.changed() => {
                        if !*self.watch.access.borrow() { self.closed = true; return Ok(None); }
                    },
                }
                tokio::time::sleep(std::time::Duration::from_millis(16)).await;
            } else {
                self.closed = true;
                return Ok(None);
            }
        }
        let page = self.watch.read(self.cursor, self.max_bytes).await?;
        if !*self.watch.access.borrow() {
            self.closed = true;
            return Ok(None);
        }
        self.cursor = Some(page.next_cursor);
        self.waiting = !page.has_more && page.resource.state == agena_domain::ContentState::Active;
        self.closed = !page.has_more && page.resource.state != agena_domain::ContentState::Active;
        Ok(Some(page))
    }
}

impl ContentWatch {
    pub fn delivery(self, mut after: Option<ContentCursor>, max_bytes: usize) -> ContentDelivery {
        if after.is_none()
            && matches!(
                self.resource.kind,
                agena_domain::ContentKind::Text
                    | agena_domain::ContentKind::Structured
                    | agena_domain::ContentKind::Document
            )
        {
            after = Some(ContentCursor {
                sequence: 0,
                ..self.resource.cursor
            });
        }
        ContentDelivery {
            watch: self,
            cursor: after,
            max_bytes: max_bytes.clamp(1, 1024 * 1024),
            waiting: false,
            closed: false,
        }
    }
    pub async fn read(
        &self,
        after: Option<ContentCursor>,
        max_bytes: usize,
    ) -> Result<ContentPage, ApplicationError> {
        if !*self.access.borrow() {
            return Err(ApplicationError::not_found(
                "The content resource is unavailable.",
            ));
        }
        let page = self
            .hub
            .read(self.resource.resource_id, after, max_bytes)
            .await
            .map_err(content_error)?;
        if !*self.access.borrow() {
            return Err(ApplicationError::not_found(
                "The content resource is unavailable.",
            ));
        }
        Ok(page)
    }
}

impl Application {
    pub async fn read_content_text(
        &self,
        params: agena_api::content::ReadContentTextParams,
    ) -> Result<agena_domain::ContentTextPage, ApplicationError> {
        let watch = self
            .watch_content(params.session_id, params.resource_id)
            .await?;
        let page = watch
            .hub
            .read_text_page(params.resource_id, params.position, params.max_bytes)
            .await
            .map_err(content_error)?;
        if !*watch.access.borrow() {
            return Err(ApplicationError::not_found(
                "The content resource is unavailable.",
            ));
        }
        Ok(page)
    }

    pub async fn read_content(
        &self,
        session_id: i64,
        id: ContentId,
        after: Option<ContentCursor>,
        max_bytes: usize,
    ) -> Result<ContentPage, ApplicationError> {
        self.watch_content(session_id, id)
            .await?
            .read(after, max_bytes)
            .await
    }

    pub async fn watch_content(
        &self,
        session_id: i64,
        id: ContentId,
    ) -> Result<ContentWatch, ApplicationError> {
        let store = self.session_store_facade()?;
        let hub = store.contents().clone();
        let resource = hub.describe(id).await.map_err(content_error)?;
        // Both change subscriptions precede authorization/bootstrap reads.
        // Rare membership changes revoke access without a query per chunk.
        let changes = hub.subscribe(id);
        let (access_tx, access) = watch::channel(true);
        let part_id = resource.part_id;
        let referenced_resource = resource.clone();
        let membership = store.subscribe(
            session_id,
            std::sync::Arc::new(move |change| {
                let revoked = match change {
                    SessionChange::SessionDeleted { .. } => true,
                    SessionChange::PartRemoved {
                        part_id: removed, ..
                    } => removed == part_id,
                    SessionChange::PartUpdated { part, .. } if part.part_id == part_id => {
                        !part_references(&part, &referenced_resource)
                    }
                    _ => false,
                };
                if revoked {
                    access_tx.send_replace(false);
                }
            }),
        );
        let view = store
            .load_part_ids(session_id, &[part_id])
            .await
            .map_err(content_error)?;
        if !view
            .parts
            .iter()
            .any(|part| part_references(part, &resource))
            || !*access.borrow()
        {
            return Err(ApplicationError::not_found(
                "The content resource is unavailable.",
            ));
        }
        Ok(ContentWatch {
            resource,
            changes,
            access,
            hub,
            _membership: membership,
        })
    }
}

fn part_references(part: &Part, resource: &ContentResource) -> bool {
    part.visibility.visible_to_user() && part.references_content(resource)
}

fn content_error(error: agena_storage::store::StoreError) -> ApplicationError {
    match error {
        agena_storage::store::StoreError::Constraint(_) => {
            ApplicationError::bad_request_error(&error)
        }
        agena_storage::store::StoreError::NotFound(_) => {
            ApplicationError::not_found("The content resource is unavailable.")
        }
        agena_storage::store::StoreError::Conflict(_) => {
            ApplicationError::conflict("The content cursor is no longer valid.")
        }
        other => ApplicationError::internal_error(&other),
    }
}
