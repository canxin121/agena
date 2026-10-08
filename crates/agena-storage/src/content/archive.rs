//! Portable resource snapshots preserve retained ranges and loss separately.

use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_ARCHIVE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentArchive {
    pub resource: ContentResource,
    pub chunks: Vec<ContentChunk>,
}

impl ContentArchive {
    pub fn validate(&self) -> Result<(), StoreError> {
        let resource = &self.resource;
        if resource.state == ContentState::Active
            || resource.committed_cursor != resource.cursor
            || resource.owner_session_id <= 0
            || resource.part_id <= 0
            || resource.dropped_bytes > resource.total_bytes
        {
            return Err(StoreError::Constraint(
                "content archive must be a sealed snapshot with durable ownership".into(),
            ));
        }
        let mut previous = 0;
        let mut bytes = 0usize;
        let mut ranges = Vec::new();
        let mut terminal = None;
        let mut document = None;
        for chunk in &self.chunks {
            chunk
                .payload
                .validate()
                .map_err(|error| StoreError::Constraint(error.into()))?;
            if chunk.cursor.epoch != resource.cursor.epoch
                || chunk.cursor.sequence <= previous
                || chunk.cursor.sequence > resource.cursor.sequence
                || !resource.kind.accepts(chunk.payload.kind())
            {
                return Err(StoreError::Constraint(
                    "invalid content archive record order or type".into(),
                ));
            }
            match &chunk.payload {
                ContentPayload::StructuredSnapshot { document: snapshot } => {
                    document = Some((chunk.cursor, snapshot.clone()))
                }
                ContentPayload::Structured { base_cursor, event } => {
                    let (cursor, previous) = document.as_ref().ok_or_else(|| {
                        StoreError::Constraint("document archive is missing its checkpoint".into())
                    })?;
                    if cursor != base_cursor {
                        return Err(StoreError::Constraint(
                            "document archive mutation is missing its base".into(),
                        ));
                    }
                    document = Some((
                        chunk.cursor,
                        previous
                            .updated(event)
                            .map_err(|error| StoreError::Constraint(error.into()))?,
                    ));
                }
                ContentPayload::Terminal { screen } => {
                    terminal = Some((
                        chunk.cursor,
                        screen.rows,
                        screen.cols,
                        screen.alternate_screen,
                    ))
                }
                ContentPayload::TerminalPatch {
                    base_cursor,
                    screen,
                    ..
                } => {
                    if terminal
                        != Some((
                            *base_cursor,
                            screen.rows,
                            screen.cols,
                            screen.alternate_screen,
                        ))
                    {
                        return Err(StoreError::Constraint(
                            "content archive patch is missing its screen base".into(),
                        ));
                    }
                    terminal = Some((
                        chunk.cursor,
                        screen.rows,
                        screen.cols,
                        screen.alternate_screen,
                    ));
                }
                _ => {}
            }
            previous = chunk.cursor.sequence;
            extend_ranges(&mut ranges, previous);
            bytes = bytes.saturating_add(chunk.payload.byte_len() + RECORD_OVERHEAD);
            if bytes > MAX_ARCHIVE_BYTES {
                return Err(StoreError::Constraint(
                    "content archive exceeds its byte budget".into(),
                ));
            }
        }
        if ranges != resource.retained_ranges {
            return Err(StoreError::Constraint(
                "content archive ranges differ from its records".into(),
            ));
        }
        Ok(())
    }

    pub fn remap(&mut self, id: ContentId, owner_session_id: i64, part_id: i64) {
        let epoch = id.0;
        self.resource.resource_id = id;
        self.resource.owner_session_id = owner_session_id;
        self.resource.part_id = part_id;
        self.resource.cursor.epoch = epoch;
        self.resource.committed_cursor.epoch = epoch;
        for chunk in &mut self.chunks {
            chunk.cursor.epoch = epoch;
            if let ContentPayload::TerminalPatch { base_cursor, .. }
            | ContentPayload::Structured { base_cursor, .. } = &mut chunk.payload
            {
                base_cursor.epoch = epoch;
            }
        }
    }
}

impl ContentHub {
    pub(crate) async fn import_guard(&self) -> tokio::sync::RwLockReadGuard<'_, ()> {
        self.0.lifecycle.read().await
    }
    pub(crate) async fn collection_guard(&self) -> tokio::sync::RwLockWriteGuard<'_, ()> {
        self.0.lifecycle.write().await
    }

    pub async fn export_snapshot(&self, id: ContentId) -> Result<ContentArchive, StoreError> {
        let mut resource = self.describe(id).await?;
        let mut cursor = ContentCursor {
            sequence: 0,
            ..resource.cursor
        };
        let mut chunks = Vec::new();
        let mut bytes = 0usize;
        while cursor.sequence < resource.cursor.sequence {
            let page = self.read(id, Some(cursor), 1024 * 1024).await?;
            for chunk in page
                .chunks
                .into_iter()
                .take_while(|chunk| chunk.cursor.sequence <= resource.cursor.sequence)
            {
                bytes = bytes.saturating_add(chunk.payload.byte_len() + RECORD_OVERHEAD);
                if bytes > MAX_ARCHIVE_BYTES {
                    return Err(StoreError::Constraint(
                        "content export exceeds its byte budget".into(),
                    ));
                }
                chunks.push(chunk);
            }
            if page.next_cursor.sequence <= cursor.sequence {
                return Err(StoreError::Conflict(
                    "content export did not advance".into(),
                ));
            }
            cursor = page.next_cursor;
        }
        // A retained range may begin with patches whose screen base was evicted.
        // Keep the next complete snapshot and its descendants.
        let mut terminal = None;
        let mut document = None;
        chunks.retain(|chunk| match &chunk.payload {
            ContentPayload::StructuredSnapshot { document: snapshot } => {
                document = Some((chunk.cursor, snapshot.clone()));
                true
            }
            ContentPayload::Structured { base_cursor, event } => match document
                .as_ref()
                .filter(|(cursor, _)| cursor == base_cursor)
                .and_then(|(_, document)| document.updated(event).ok())
            {
                Some(next) => {
                    document = Some((chunk.cursor, next));
                    true
                }
                None => {
                    document = None;
                    false
                }
            },
            ContentPayload::Terminal { screen } => {
                terminal = Some((
                    chunk.cursor,
                    screen.rows,
                    screen.cols,
                    screen.alternate_screen,
                ));
                true
            }
            ContentPayload::TerminalPatch {
                base_cursor,
                screen,
                ..
            } => {
                if terminal
                    != Some((
                        *base_cursor,
                        screen.rows,
                        screen.cols,
                        screen.alternate_screen,
                    ))
                {
                    return false;
                }
                terminal = Some((
                    chunk.cursor,
                    screen.rows,
                    screen.cols,
                    screen.alternate_screen,
                ));
                true
            }
            _ => true,
        });
        resource.retained_ranges.clear();
        for chunk in &chunks {
            extend_ranges(&mut resource.retained_ranges, chunk.cursor.sequence);
        }
        if resource.state == ContentState::Active {
            resource.state = ContentState::Interrupted;
        }
        resource.committed_cursor = resource.cursor;
        let archive = ContentArchive { resource, chunks };
        archive.validate()?;
        Ok(archive)
    }

    pub async fn restore(&self, archive: &ContentArchive) -> Result<(), StoreError> {
        archive.validate()?;
        self.0.backend.restore(archive).await
    }

    pub async fn delete(&self, id: ContentId) -> Result<(), StoreError> {
        if self.subscribe(id).is_some() {
            return Err(StoreError::Conflict(
                "cannot delete an active content source".into(),
            ));
        }
        self.0.backend.delete(id).await
    }

    pub async fn collect_orphans(
        &self,
        mut referenced: HashSet<ContentId>,
    ) -> Result<usize, StoreError> {
        referenced.extend(
            self.0
                .active
                .lock()
                .expect("content registry lock")
                .keys()
                .copied(),
        );
        self.0.backend.prune(&referenced).await
    }
}
