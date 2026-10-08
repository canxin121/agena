//! Factual, bounded Part queries shared by HTTP, WS and IPC.

use crate::{
    Application, ApplicationError,
    pagination::{SessionPartCursor, decode_cursor, encode_cursor},
};
use agena_api::{
    live::{RunSummaryResource, SessionPartsResource, ToolDetailSection},
    queries::{ReadPartsParams, ReadRunsParams},
};
use agena_storage::store::PartCursor;

pub(crate) fn decode_part_cursor(
    session_id: i64,
    cursor: Option<&str>,
) -> Result<Option<PartCursor>, ApplicationError> {
    let value = cursor.map(decode_cursor::<SessionPartCursor>).transpose()?;
    if value.is_some_and(|cursor| cursor.session_id != session_id) {
        return Err(ApplicationError::bad_request(
            "The page cursor belongs to a different session.",
        ));
    }
    Ok(value.map(|cursor| PartCursor {
        created_at_ms: cursor.created_at_ms,
        part_id: cursor.part_id,
    }))
}

pub(crate) fn encode_part_cursor(
    session_id: i64,
    cursor: Option<PartCursor>,
) -> Result<Option<String>, ApplicationError> {
    cursor
        .map(|cursor| {
            encode_cursor(&SessionPartCursor {
                session_id,
                created_at_ms: cursor.created_at_ms,
                part_id: cursor.part_id,
            })
        })
        .transpose()
}

fn requested_sections(sections: &[ToolDetailSection]) -> &[ToolDetailSection] {
    if sections.is_empty() {
        &[ToolDetailSection::Presentation]
    } else {
        sections
    }
}

impl Application {
    pub async fn read_parts(
        &self,
        request: ReadPartsParams,
    ) -> Result<SessionPartsResource, ApplicationError> {
        if request.ids.len() > 256
            || request.run_ids.len() > 32
            || request
                .ids
                .iter()
                .chain(&request.run_ids)
                .any(|id| *id <= 0)
        {
            return Err(ApplicationError::bad_request(
                "The Part selection is invalid or exceeds its limit.",
            ));
        }
        if !request.ids.is_empty()
            && (!request.run_ids.is_empty() || request.cursor.is_some() || request.limit.is_some())
        {
            return Err(ApplicationError::bad_request(
                "ids cannot be combined with a range selection.",
            ));
        }
        let store = self.session_store_facade()?;
        let (meta, mut raw, next, has_more) = if request.ids.is_empty() {
            let page = store
                .load_visible_part_window(
                    request.session_id,
                    &request.run_ids,
                    decode_part_cursor(request.session_id, request.cursor.as_deref())?,
                    request.limit.unwrap_or(64).clamp(1, 256) as usize,
                )
                .await
                .map_err(store_error)?;
            let next = page.parts.last().map(|p| PartCursor {
                created_at_ms: p.created_at_ms,
                part_id: p.part_id,
            });
            (page.meta, page.parts, next, page.has_more)
        } else {
            let view = store
                .load_part_ids(request.session_id, &request.ids)
                .await
                .map_err(store_error)?;
            (view.meta, view.parts, None, false)
        };
        raw.retain(|p| p.visibility.visible_to_user());
        raw.sort_unstable_by_key(|p| (p.created_at_ms, p.part_id));
        let mut parts = Vec::with_capacity(raw.len());
        for part in &raw {
            parts.push(
                self.project_part(part, requested_sections(&request.sections))
                    .await,
            );
        }
        let user_ids = parts
            .iter()
            .filter(|p| p.kind == "run" && p.role == "user")
            .map(|p| p.part_id)
            .collect::<Vec<_>>();
        let ordinals = store
            .user_message_ordinals(request.session_id, &user_ids)
            .await
            .map_err(store_error)?;
        for part in &mut parts {
            part.user_message_ordinal = ordinals.get(&part.part_id).copied();
        }
        Ok(SessionPartsResource {
            session_id: request.session_id,
            version: meta.version,
            page: agena_api::pagination::PageInfo {
                returned: parts.len() as u64,
                has_more,
                next_cursor: encode_part_cursor(request.session_id, next)?,
            },
            parts,
            runs: Vec::new(),
            user_message_count: None,
        })
    }

    pub async fn read_runs(
        &self,
        request: ReadRunsParams,
    ) -> Result<SessionPartsResource, ApplicationError> {
        let store = self.session_store_facade()?;
        let page = store
            .load_run_window(
                request.session_id,
                decode_part_cursor(request.session_id, request.cursor.as_deref())?,
                request.limit.unwrap_or(8).clamp(1, 32) as usize,
                request.part_limit.unwrap_or(32).clamp(1, 128) as usize,
            )
            .await
            .map_err(store_error)?;
        let mut parts = Vec::with_capacity(page.parts.len());
        for part in &page.parts {
            parts.push(
                self.project_part(part, requested_sections(&request.sections))
                    .await,
            );
        }
        let user_ids = parts
            .iter()
            .filter(|p| p.kind == "run" && p.role == "user")
            .map(|p| p.part_id)
            .collect::<Vec<_>>();
        let ordinals = store
            .user_message_ordinals(request.session_id, &user_ids)
            .await
            .map_err(store_error)?;
        for part in &mut parts {
            part.user_message_ordinal = ordinals.get(&part.part_id).copied();
        }
        let mut runs = Vec::with_capacity(page.runs.len());
        for run in page.runs {
            runs.push(RunSummaryResource {
                run_id: run.run_id,
                part_count: run.part_count,
                page: agena_api::pagination::PageInfo {
                    returned: run.loaded_count,
                    has_more: run.loaded_count < run.part_count,
                    next_cursor: encode_part_cursor(request.session_id, run.next_cursor)?,
                },
            });
        }
        let count = store
            .user_message_count(request.session_id)
            .await
            .map_err(store_error)?;
        Ok(SessionPartsResource {
            session_id: request.session_id,
            version: page.meta.version,
            page: agena_api::pagination::PageInfo {
                returned: runs.len() as u64,
                has_more: page.has_more,
                next_cursor: encode_part_cursor(request.session_id, page.next_cursor)?,
            },
            parts,
            runs,
            user_message_count: Some(count),
        })
    }
}

fn store_error(error: agena_storage::store::StoreError) -> ApplicationError {
    match error {
        agena_storage::store::StoreError::NotFound(_) => {
            ApplicationError::not_found("The session was not found.")
        }
        agena_storage::store::StoreError::Conflict(_) => {
            ApplicationError::conflict("The Part selection is no longer valid.")
        }
        error => ApplicationError::internal_error(&error),
    }
}
