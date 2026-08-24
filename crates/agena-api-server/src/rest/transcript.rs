//! Server-side transcript presentation paging.
//!
//! The server reads durable raw parts internally, but public transcript
//! responses use the same presentation-safe tool projection as `/parts`:
//! collapsed input/metadata/output sections stay server-side until a client
//! explicitly requests one. The chat timeline walks raw pages inside the
//! server, folds assistant activity there, and sends only the visible tail
//! plus an opaque fold expansion cursor to clients.

use std::collections::HashMap;

use agena_api::live::{SessionPartsResource, SessionTranscriptFoldResource};
use agena_storage::store::{Part, PartCursor, SessionStore};
use axum::{
    Json,
    extract::{Path, Query as AxumQuery, State},
};
use serde::Deserialize;

use crate::{error::ServerError, live::project_parts_for_user, state::AppState};

const RAW_SCAN_PAGE_SIZE: i64 = 200;
const ACTIVITY_VISIBLE_TAIL: usize = 5;
const MAX_ACTIVITY_VISIBLE_TAIL: usize = 50;
/// A history page contains one user-side role group and one assistant-side
/// role group. Consecutive runs of the same role stay together so a page
/// never cuts a burst of user sends or assistant continuations in half.
const DEFAULT_MESSAGE_ROLE_GROUPS: usize = 2;
const MAX_VISIBLE_BLOCKS: usize = 12;

fn transcript_visible_to_user(part: &Part) -> bool {
    part.visibility.visible_to_user()
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionTranscriptQuery {
    #[serde(default)]
    pub limit: Option<u64>,
    /// Number of assistant activity parts kept visible before a fold is made.
    /// The client may change this independently from the logical block limit.
    #[serde(default)]
    pub activity_limit: Option<u64>,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionTranscriptRunQuery {
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone)]
struct RunUnit {
    run_id: Option<i64>,
    role: String,
    parts: Vec<Part>,
}

#[derive(Debug, Clone)]
struct LogicalBlock {
    role: String,
    units: Vec<RunUnit>,
}

impl LogicalBlock {
    fn start(&self) -> Option<&Part> {
        self.units
            .iter()
            .flat_map(|unit| unit.parts.iter())
            .min_by_key(|part| (part.created_at_ms, part.part_id))
    }
}

#[derive(Debug, Clone)]
struct VisibleProjection {
    parts: Vec<Part>,
    folds: Vec<SessionTranscriptFoldResource>,
}

pub async fn list_session_transcript(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<SessionTranscriptQuery>,
) -> Result<impl axum::response::IntoResponse, ServerError> {
    let store = state.session_store()?;
    let user_message_count = store
        .user_message_count(session_id)
        .await
        .map_err(|error| ServerError::internal_error(&error))?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_MESSAGE_ROLE_GROUPS as u64)
        .clamp(1, MAX_VISIBLE_BLOCKS as u64) as usize;
    let activity_limit = query
        .activity_limit
        .unwrap_or(ACTIVITY_VISIBLE_TAIL as u64)
        .clamp(1, MAX_ACTIVITY_VISIBLE_TAIL as u64) as usize;
    let cursor = decode_cursor(query.cursor.as_deref(), session_id)?;
    let page = load_visible_page(store.as_ref(), session_id, cursor, limit, activity_limit).await?;
    let next_cursor = page
        .next_cursor
        .map(|cursor| encode_transcript_cursor(session_id, cursor))
        .transpose()?;

    let projected = project_parts_for_user(&state, &page.parts).await;
    Ok(Json(SessionPartsResource {
        session_id,
        version: page.version,
        parts: projected,
        folds: page.folds,
        user_message_count: Some(user_message_count),
        page: agena_api::pagination::PageInfo {
            next_cursor,
            has_more: page.has_more,
            returned: page.parts.len() as u64,
        },
    }))
}

pub async fn list_session_transcript_run_parts(
    State(state): State<AppState>,
    Path((session_id, run_id)): Path<(i64, i64)>,
    AxumQuery(query): AxumQuery<SessionTranscriptRunQuery>,
) -> Result<impl axum::response::IntoResponse, ServerError> {
    let store = state.session_store()?;
    let limit = query.limit.unwrap_or(5).clamp(1, 50);
    let cursor = query
        .cursor
        .as_deref()
        .map(|value| decode_run_cursor(value, session_id))
        .transpose()?;
    let page = store
        .load_run_page(session_id, run_id, cursor, limit as i64)
        .await
        .map_err(|error| ServerError::internal_error(&error))?;
    let next_cursor = page
        .parts
        .last()
        .map(|part| PartCursor {
            created_at_ms: part.created_at_ms,
            part_id: part.part_id,
        })
        .map(|cursor| encode_run_cursor(session_id, cursor))
        .transpose()?;
    let mut parts = page
        .parts
        .into_iter()
        .filter(transcript_visible_to_user)
        .collect::<Vec<_>>();
    parts.reverse();

    let projected = project_parts_for_user(&state, &parts).await;
    Ok(Json(SessionPartsResource {
        session_id,
        version: page.meta.version,
        parts: projected,
        folds: Vec::new(),
        user_message_count: None,
        page: agena_api::pagination::PageInfo {
            next_cursor,
            has_more: page.has_more,
            returned: parts.len() as u64,
        },
    }))
}

pub async fn list_session_transcript_fold_parts(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<SessionTranscriptRunQuery>,
) -> Result<impl axum::response::IntoResponse, ServerError> {
    let store = state.session_store()?;
    let limit = query.limit.unwrap_or(5).clamp(1, 50) as usize;
    let cursor = query
        .cursor
        .as_deref()
        .map(|value| decode_fold_cursor(value, session_id))
        .transpose()?;
    let Some(cursor) = cursor else {
        return Err(ServerError::bad_request(
            "A folded transcript cursor is required.",
        ));
    };
    let before = Some(PartCursor {
        created_at_ms: cursor.created_at_ms,
        part_id: cursor.part_id,
    });
    let mut candidates = Vec::new();
    let mut has_more = false;
    for run_id in &cursor.run_ids {
        let page = store
            .load_run_page(session_id, *run_id, before, limit as i64)
            .await
            .map_err(|error| ServerError::internal_error(&error))?;
        has_more |= page.has_more;
        candidates.extend(page.parts);
    }
    candidates.sort_by_key(|part| (part.created_at_ms, part.part_id));
    candidates.dedup_by_key(|part| part.part_id);
    let take_from = candidates.len().saturating_sub(limit);
    let raw_selected = candidates.split_off(take_from);
    has_more |= take_from > 0;
    let next_cursor = raw_selected.first().map(|part| {
        encode_fold_cursor(
            session_id,
            &cursor.run_ids,
            PartCursor {
                created_at_ms: part.created_at_ms,
                part_id: part.part_id,
            },
        )
    });
    let selected = raw_selected
        .into_iter()
        .filter(transcript_visible_to_user)
        .collect::<Vec<_>>();
    let projected = project_parts_for_user(&state, &selected).await;
    Ok(Json(SessionPartsResource {
        session_id,
        version: store
            .load_page(session_id, None, 1)
            .await
            .map_err(|error| ServerError::internal_error(&error))?
            .meta
            .version,
        parts: projected,
        folds: Vec::new(),
        user_message_count: None,
        page: agena_api::pagination::PageInfo {
            next_cursor: next_cursor.transpose()?,
            has_more,
            returned: selected.len() as u64,
        },
    }))
}

struct VisiblePage {
    version: i64,
    parts: Vec<Part>,
    folds: Vec<SessionTranscriptFoldResource>,
    next_cursor: Option<PartCursor>,
    has_more: bool,
}

async fn load_visible_page(
    store: &dyn SessionStore,
    session_id: i64,
    before: Option<PartCursor>,
    limit: usize,
    activity_visible_tail: usize,
) -> Result<VisiblePage, ServerError> {
    let mut raw_desc = Vec::<Part>::new();
    let mut raw_before = before;
    let mut raw_has_more = true;
    let mut version = 0;
    let mut blocks = Vec::new();

    // When the raw page ends in the middle of a same-role burst, the current
    // block list can reach `limit` before that burst is complete. Scan one
    // additional logical block whenever older raw parts remain; otherwise a
    // consecutive user/assistant run could be split across transcript pages.
    while raw_has_more && blocks.len() <= limit {
        let page = store
            .load_page(session_id, raw_before, RAW_SCAN_PAGE_SIZE)
            .await
            .map_err(|error| ServerError::internal_error(&error))?;
        version = page.meta.version;
        raw_has_more = page.has_more;
        let oldest = page.parts.last().map(|part| PartCursor {
            created_at_ms: part.created_at_ms,
            part_id: part.part_id,
        });
        raw_desc.extend(page.parts.into_iter().filter(transcript_visible_to_user));
        let mut chronological = raw_desc.clone();
        chronological.reverse();
        blocks = logical_blocks(&chronological);
        raw_before = oldest;
        if oldest.is_none() {
            break;
        }
    }

    if blocks.is_empty() {
        return Ok(VisiblePage {
            version,
            parts: Vec::new(),
            folds: Vec::new(),
            next_cursor: None,
            has_more: false,
        });
    }

    let selected_start = blocks.len().saturating_sub(limit);
    let mut selected = blocks.split_off(selected_start);
    let next_cursor = selected
        .first()
        .and_then(LogicalBlock::start)
        .map(|part| PartCursor {
            created_at_ms: part.created_at_ms,
            part_id: part.part_id,
        });

    for block in &mut selected {
        for unit in &mut block.units {
            let Some(run_id) = unit.run_id else {
                continue;
            };
            let marker = unit.parts.iter().find(|part| part.kind == "run").cloned();
            let mut full = load_all_run_parts(store, session_id, run_id).await?;
            if let Some(marker) = marker {
                full.push(marker);
            }
            if !full.is_empty() {
                full.sort_by_key(|part| (part.created_at_ms, part.part_id));
                unit.parts = full;
            }
        }
    }

    let projection = project_visible_blocks(session_id, &selected, activity_visible_tail)?;
    Ok(VisiblePage {
        version,
        parts: projection.parts,
        folds: projection.folds,
        next_cursor,
        has_more: raw_has_more || selected_start > 0,
    })
}

async fn load_all_run_parts(
    store: &dyn SessionStore,
    session_id: i64,
    run_id: i64,
) -> Result<Vec<Part>, ServerError> {
    let mut before = None;
    let mut newest_first = Vec::new();
    loop {
        let page = store
            .load_run_page(session_id, run_id, before, RAW_SCAN_PAGE_SIZE)
            .await
            .map_err(|error| ServerError::internal_error(&error))?;
        let next_before = page.parts.last().map(|part| PartCursor {
            created_at_ms: part.created_at_ms,
            part_id: part.part_id,
        });
        newest_first.extend(page.parts);
        if !page.has_more || next_before.is_none() {
            break;
        }
        before = next_before;
    }
    newest_first.retain(transcript_visible_to_user);
    newest_first.reverse();
    Ok(newest_first)
}

fn logical_blocks(parts: &[Part]) -> Vec<LogicalBlock> {
    let mut marker_indexes = HashMap::<i64, usize>::new();
    let mut units = Vec::<RunUnit>::new();

    for part in parts {
        if part.kind == "run" {
            let index = units.len();
            marker_indexes.insert(part.part_id, index);
            units.push(RunUnit {
                run_id: Some(part.part_id),
                role: part.role.as_str().to_owned(),
                parts: vec![part.clone()],
            });
        }
    }

    for part in parts.iter().filter(|part| part.kind != "run") {
        let Some(run_id) = part.run_id else {
            continue;
        };
        let Some(index) = marker_indexes.get(&run_id).copied() else {
            continue;
        };
        units[index].parts.push(part.clone());
    }

    // Marker order is canonical. Content is already ordered by the raw part
    // scan, and every content part has exactly one durable marker owner.
    units.sort_by_key(|unit| {
        unit.parts
            .iter()
            .map(|part| (part.created_at_ms, part.part_id))
            .min()
            .unwrap_or((i64::MAX, i64::MAX))
    });

    let mut blocks = Vec::<LogicalBlock>::new();
    for unit in units {
        if matches!(unit.role.as_str(), "assistant" | "user")
            && blocks.last().is_some_and(|block| block.role == unit.role)
        {
            blocks
                .last_mut()
                .expect("same-role block exists")
                .units
                .push(unit);
        } else {
            blocks.push(LogicalBlock {
                role: unit.role.clone(),
                units: vec![unit],
            });
        }
    }
    blocks
}

fn project_visible_blocks(
    session_id: i64,
    blocks: &[LogicalBlock],
    activity_visible_tail: usize,
) -> Result<VisibleProjection, ServerError> {
    let mut parts = Vec::new();
    let mut folds = Vec::new();
    for block in blocks {
        for unit in &block.units {
            let mut content = unit.parts.clone();
            content.sort_by_key(|part| (part.created_at_ms, part.part_id));
            if block.role != "assistant" {
                parts.extend(content);
            }
        }
        if block.role == "assistant" {
            let (visible, unit_folds) =
                visible_assistant_block(session_id, block, activity_visible_tail)?;
            parts.extend(visible);
            folds.extend(unit_folds);
        }
    }
    parts = parts.into_iter().map(compact_presentation_part).collect();
    parts.sort_by_key(|part| (part.created_at_ms, part.part_id));
    Ok(VisibleProjection { parts, folds })
}

/// Run markers may carry provider-internal round/history arrays that are
/// useful to the model but not to the collapsed transcript header. Never send
/// those payloads merely because a marker is needed to attach a visible tail.
fn compact_presentation_part(mut part: Part) -> Part {
    if part.kind != "run" {
        return part;
    }
    let Some(object) = part.content.as_object() else {
        return part;
    };
    let keep = [
        "run_kind",
        "abort_reason",
        "provider_id",
        "adapter_id",
        "model_id",
        "turn_id",
        "reply_id",
    ];
    part.content = object
        .iter()
        .filter(|(key, _)| keep.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<serde_json::Map<_, _>>()
        .into();
    part
}

fn visible_assistant_block(
    session_id: i64,
    block: &LogicalBlock,
    activity_visible_tail: usize,
) -> Result<(Vec<Part>, Vec<SessionTranscriptFoldResource>), ServerError> {
    let run_ids = block
        .units
        .iter()
        .filter_map(|unit| unit.run_id)
        .collect::<Vec<_>>();
    let mut markers = Vec::new();
    let mut activities = Vec::new();
    for unit in &block.units {
        for part in &unit.parts {
            if part.kind == "run" {
                markers.push(part.clone());
            } else {
                activities.push(part.clone());
            }
        }
    }
    markers.sort_by_key(|part| (part.created_at_ms, part.part_id));
    activities.sort_by_key(|part| (part.created_at_ms, part.part_id));
    let hidden_count = activities.len().saturating_sub(activity_visible_tail);
    let visible_start = hidden_count;
    let mut visible = markers;
    visible.extend(activities[visible_start..].iter().cloned());
    visible.sort_by_key(|part| (part.created_at_ms, part.part_id));
    if hidden_count == 0 || run_ids.is_empty() {
        return Ok((visible, Vec::new()));
    }
    let anchor = &activities[visible_start];
    let Some(anchor_run_id) = anchor.run_id else {
        return Ok((visible, Vec::new()));
    };
    let next_cursor = Some(encode_fold_cursor(
        session_id,
        &run_ids,
        PartCursor {
            created_at_ms: anchor.created_at_ms,
            part_id: anchor.part_id,
        },
    )?);
    Ok((
        visible,
        vec![SessionTranscriptFoldResource {
            run_id: anchor_run_id,
            run_ids,
            anchor_part_id: anchor.part_id,
            hidden_count: hidden_count as u64,
            next_cursor,
        }],
    ))
}

fn decode_cursor(value: Option<&str>, session_id: i64) -> Result<Option<PartCursor>, ServerError> {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let cursor = agena_application::pagination::decode_cursor::<
        agena_application::pagination::SessionTranscriptCursor,
    >(value)
    .map_err(crate::rest::server_error_from_application)?;
    if cursor.session_id != session_id {
        return Err(ServerError::bad_request(
            "The transcript cursor belongs to a different session.",
        ));
    }
    Ok(Some(PartCursor {
        created_at_ms: cursor.created_at_ms,
        part_id: cursor.part_id,
    }))
}

fn encode_transcript_cursor(session_id: i64, cursor: PartCursor) -> Result<String, ServerError> {
    agena_application::pagination::encode_cursor(
        &agena_application::pagination::SessionTranscriptCursor {
            session_id,
            created_at_ms: cursor.created_at_ms,
            part_id: cursor.part_id,
        },
    )
    .map_err(crate::rest::server_error_from_application)
}

fn decode_run_cursor(value: &str, session_id: i64) -> Result<PartCursor, ServerError> {
    let cursor = agena_application::pagination::decode_cursor::<
        agena_application::pagination::SessionPartCursor,
    >(value)
    .map_err(crate::rest::server_error_from_application)?;
    if cursor.session_id != session_id {
        return Err(ServerError::bad_request(
            "The transcript run cursor belongs to a different session.",
        ));
    }
    Ok(PartCursor {
        created_at_ms: cursor.created_at_ms,
        part_id: cursor.part_id,
    })
}

fn encode_run_cursor(session_id: i64, cursor: PartCursor) -> Result<String, ServerError> {
    agena_application::pagination::encode_cursor(
        &agena_application::pagination::SessionPartCursor {
            session_id,
            created_at_ms: cursor.created_at_ms,
            part_id: cursor.part_id,
        },
    )
    .map_err(crate::rest::server_error_from_application)
}

fn decode_fold_cursor(
    value: &str,
    session_id: i64,
) -> Result<agena_application::pagination::SessionTranscriptFoldCursor, ServerError> {
    let cursor = agena_application::pagination::decode_cursor::<
        agena_application::pagination::SessionTranscriptFoldCursor,
    >(value)
    .map_err(crate::rest::server_error_from_application)?;
    if cursor.session_id != session_id || cursor.run_ids.is_empty() {
        return Err(ServerError::bad_request(
            "The folded transcript cursor is invalid for this session.",
        ));
    }
    Ok(cursor)
}

fn encode_fold_cursor(
    session_id: i64,
    run_ids: &[i64],
    cursor: PartCursor,
) -> Result<String, ServerError> {
    agena_application::pagination::encode_cursor(
        &agena_application::pagination::SessionTranscriptFoldCursor {
            session_id,
            run_ids: run_ids.to_vec(),
            created_at_ms: cursor.created_at_ms,
            part_id: cursor.part_id,
        },
    )
    .map_err(crate::rest::server_error_from_application)
}

#[cfg(test)]
mod tests {
    use agena_storage::store::{PartRole, PartState, PartVisibility};

    use super::*;

    fn part(id: i64, kind: &str) -> Part {
        Part {
            part_id: id,
            kind: kind.to_owned(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content: serde_json::json!({}),
            summary: None,
            visibility: agena_storage::store::PartVisibility::Both,
            parent_part_id: None,
            run_id: Some(100),
            origin_session_id: 1,
            revision: 1,
            started_at_ms: id,
            finished_at_ms: Some(id),
            created_at_ms: id,
            updated_at_ms: id,
            provider_state: None,
        }
    }

    #[test]
    fn server_fold_keeps_only_the_activity_tail_for_a_seven_part_sequence() {
        let parts = (1..=7).map(|id| part(id, "tool_call")).collect::<Vec<_>>();
        let block = LogicalBlock {
            role: "assistant".to_owned(),
            units: vec![RunUnit {
                run_id: Some(100),
                role: "assistant".to_owned(),
                parts,
            }],
        };
        let (visible, folds) = visible_assistant_block(1, &block, ACTIVITY_VISIBLE_TAIL).unwrap();

        assert_eq!(
            visible.iter().map(|part| part.part_id).collect::<Vec<_>>(),
            vec![3, 4, 5, 6, 7]
        );
        assert_eq!(folds.len(), 1);
        assert_eq!(folds[0].run_id, 100);
        assert_eq!(folds[0].anchor_part_id, 3);
        assert_eq!(folds[0].hidden_count, 2);
        assert!(folds[0].next_cursor.is_some());
    }

    #[test]
    fn server_fold_honors_a_custom_activity_tail() {
        let parts = (1..=7).map(|id| part(id, "tool_call")).collect::<Vec<_>>();
        let block = LogicalBlock {
            role: "assistant".to_owned(),
            units: vec![RunUnit {
                run_id: Some(100),
                role: "assistant".to_owned(),
                parts,
            }],
        };

        let (visible, folds) = visible_assistant_block(1, &block, 2).unwrap();

        assert_eq!(
            visible.iter().map(|part| part.part_id).collect::<Vec<_>>(),
            vec![6, 7]
        );
        assert_eq!(folds[0].hidden_count, 5);
    }

    #[test]
    fn transcript_exposes_both_and_user_but_not_ai() {
        for (visibility, expected) in [
            (PartVisibility::Both, true),
            (PartVisibility::User, true),
            (PartVisibility::Ai, false),
        ] {
            let mut candidate = part(1, "text");
            candidate.visibility = visibility;
            assert_eq!(
                transcript_visible_to_user(&candidate),
                expected,
                "{visibility:?}"
            );
        }
    }

    #[test]
    fn pagination_groups_consecutive_user_and_assistant_runs_together() {
        let mut user_one = part(1, "run");
        user_one.role = PartRole::User;
        let mut user_two = part(2, "run");
        user_two.role = PartRole::User;
        let mut assistant_one = part(3, "run");
        assistant_one.role = PartRole::Assistant;
        let mut assistant_two = part(4, "run");
        assistant_two.role = PartRole::Assistant;
        let mut user_three = part(5, "run");
        user_three.role = PartRole::User;

        let blocks =
            logical_blocks(&[user_one, user_two, assistant_one, assistant_two, user_three]);

        assert_eq!(
            blocks
                .iter()
                .map(|block| block.role.as_str())
                .collect::<Vec<_>>(),
            vec!["user", "assistant", "user"]
        );
        assert_eq!(
            blocks
                .iter()
                .map(|block| block.units.len())
                .collect::<Vec<_>>(),
            vec![2, 2, 1]
        );
    }
}
