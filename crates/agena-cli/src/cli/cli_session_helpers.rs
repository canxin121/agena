use super::{AppError, BTreeMap, HashSet, SessionListView, SessionSummary};

pub(super) fn filter_session_summaries_by_view(
    sessions: Vec<SessionSummary>,
    view: SessionListView,
    anchor_session_id: Option<i64>,
) -> Result<Vec<SessionSummary>, AppError> {
    match view {
        SessionListView::Roots => {
            let mut roots = sessions
                .into_iter()
                .filter(|session| session.parent_id.is_none())
                .collect::<Vec<_>>();
            roots.sort_by(session_summary_sort_recent);
            Ok(roots)
        }
        SessionListView::All => render_session_summary_tree(sessions, None),
        SessionListView::Subtree => {
            let anchor_session_id = anchor_session_id.ok_or_else(|| {
                AppError::Config("subtree view requires --anchor-session-id <id>".to_owned())
            })?;
            render_session_summary_tree(sessions, Some(anchor_session_id))
        }
    }
}

pub(super) fn render_session_summary_tree(
    sessions: Vec<SessionSummary>,
    anchor_session_id: Option<i64>,
) -> Result<Vec<SessionSummary>, AppError> {
    let by_id = sessions
        .into_iter()
        .map(|session| (session.id, session))
        .collect::<BTreeMap<_, _>>();
    let mut children = BTreeMap::<Option<i64>, Vec<i64>>::new();
    for session in by_id.values() {
        let parent_id = session
            .parent_id
            .filter(|parent_id| by_id.contains_key(parent_id));
        children.entry(parent_id).or_default().push(session.id);
    }
    for child_ids in children.values_mut() {
        child_ids.sort_by(|left, right| session_summary_sort_recent(&by_id[left], &by_id[right]));
    }

    let root_ids = match anchor_session_id {
        Some(anchor_id) => vec![resolve_session_summary_root(anchor_id, &by_id)?],
        None => children.get(&None).cloned().unwrap_or_default(),
    };
    let kept_ids = match anchor_session_id {
        Some(root_id) => collect_session_summary_subtree_ids(
            resolve_session_summary_root(root_id, &by_id)?,
            &children,
        ),
        None => by_id.keys().copied().collect::<HashSet<_>>(),
    };
    let mut out = Vec::new();
    for root_id in root_ids {
        append_session_summary_subtree(root_id, &children, &by_id, &kept_ids, &mut out);
    }
    Ok(out)
}

pub(super) fn resolve_session_summary_root(
    session_id: i64,
    by_id: &BTreeMap<i64, SessionSummary>,
) -> Result<i64, AppError> {
    let mut current = session_id;
    let mut seen = HashSet::new();
    loop {
        let session = by_id.get(&current).ok_or_else(|| {
            AppError::Config(format!("session not found for subtree view: {session_id}"))
        })?;
        let Some(parent_id) = session.parent_id else {
            return Ok(current);
        };
        if !seen.insert(current) {
            return Err(AppError::Internal(format!(
                "cycle detected while resolving session subtree root for {session_id}"
            )));
        }
        if !by_id.contains_key(&parent_id) {
            return Ok(current);
        }
        current = parent_id;
    }
}

pub(super) fn collect_session_summary_subtree_ids(
    root_id: i64,
    children: &BTreeMap<Option<i64>, Vec<i64>>,
) -> HashSet<i64> {
    let mut kept = HashSet::new();
    let mut stack = vec![root_id];
    while let Some(session_id) = stack.pop() {
        if !kept.insert(session_id) {
            continue;
        }
        if let Some(child_ids) = children.get(&Some(session_id)) {
            stack.extend(child_ids.iter().copied());
        }
    }
    kept
}

pub(super) fn append_session_summary_subtree(
    session_id: i64,
    children: &BTreeMap<Option<i64>, Vec<i64>>,
    by_id: &BTreeMap<i64, SessionSummary>,
    kept_ids: &HashSet<i64>,
    out: &mut Vec<SessionSummary>,
) {
    if !kept_ids.contains(&session_id) {
        return;
    }
    if let Some(session) = by_id.get(&session_id) {
        out.push(session.clone());
    }
    if let Some(child_ids) = children.get(&Some(session_id)) {
        for child_id in child_ids {
            append_session_summary_subtree(*child_id, children, by_id, kept_ids, out);
        }
    }
}

pub(super) fn session_summary_sort_recent(
    left: &SessionSummary,
    right: &SessionSummary,
) -> std::cmp::Ordering {
    right
        .updated_at
        .cmp(&left.updated_at)
        .then_with(|| right.id.cmp(&left.id))
}

pub(super) fn paginate_session_summaries(
    sessions: Vec<SessionSummary>,
    offset: u64,
    limit: u64,
) -> Vec<SessionSummary> {
    sessions
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect()
}

/// Keep the CLI's durable session output separate from the API's live state.
pub(super) fn session_summary_from_resource(
    resource: agena_api::resource::SessionResource,
) -> SessionSummary {
    SessionSummary {
        id: resource.id,
        parent_id: resource.parent_id,
        depth: resource.depth,
        root_id: resource.root_id,
        workspace_id: resource.workspace_id,
        title: resource.title,
        favorite: resource.favorite,
        pinned: resource.pinned,
        version: resource.version,
        relation_kind: resource.relation_kind,
        lifecycle_state: resource.lifecycle_state,
        source_cutoff_seq_global: resource.source_cutoff_seq_global,
        source_message_id: resource.source_message_id,
        agent_name: resource.agent_name,
        agent_status: resource.agent_status,
        created_at: resource.created_at,
        updated_at: resource.updated_at,
        message_count: resource.message_count,
        child_session_count: resource.child_session_count,
        last_message_at: resource.last_message_at,
    }
}

#[cfg(test)]
mod session_projection_tests {
    use super::*;
    use serde_json::{Value, json};

    fn resource_fixture() -> Value {
        json!({
            "id": 43, "parent_id": 42, "depth": 1, "root_id": 42,
            "workspace_id": 7, "title": "界面 👩‍💻", "favorite": true,
            "pinned": true, "version": 11, "relation_kind": "subagent",
            "lifecycle_state": "ready", "is_subagent": true,
            "state": {"kind": "failed", "data": {"failure": {"private": "live-only"}}},
            "source_cutoff_seq_global": 123, "source_message_id": 41,
            "agent_name": "worker", "agent_status": "completed",
            "created_at": "2026-10-31T00:00:00Z", "updated_at": "2026-10-31T00:01:00Z",
            "message_count": u64::MAX, "child_session_count": 3,
            "last_message_at": "2026-10-31T00:00:30Z"
        })
    }

    #[test]
    fn session_projection_preserves_durable_wire_fields_without_live_state() {
        let mut expected = resource_fixture();
        let resource = serde_json::from_value(expected.clone()).unwrap();
        expected.as_object_mut().unwrap().remove("state");
        expected.as_object_mut().unwrap().remove("is_subagent");
        let summary = session_summary_from_resource(resource);
        assert_eq!(serde_json::to_value(summary).unwrap(), expected);
    }

    #[test]
    fn session_projection_keeps_missing_flags_and_optional_fields() {
        let mut value = resource_fixture();
        for key in [
            "favorite",
            "pinned",
            "source_cutoff_seq_global",
            "source_message_id",
            "agent_name",
            "agent_status",
            "last_message_at",
        ] {
            value.as_object_mut().unwrap().remove(key);
        }
        let summary = session_summary_from_resource(serde_json::from_value(value).unwrap());
        assert!(!summary.favorite && !summary.pinned);
        assert_eq!(summary.message_count, u64::MAX);
        assert!(summary.agent_name.is_none() && summary.agent_status.is_none());
        let wire = serde_json::to_value(summary).unwrap();
        for key in [
            "source_cutoff_seq_global",
            "source_message_id",
            "agent_name",
            "agent_status",
            "last_message_at",
            "state",
            "is_subagent",
        ] {
            assert!(wire.get(key).is_none(), "{key}");
        }
    }
}
