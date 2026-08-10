use super::{AppError, BTreeMap, HashSet, SessionListRequest, SessionListView, SessionSummary};

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

pub(super) async fn selected_session_id(
    queries: &dyn agena_runtime::SessionQueryService,
    session_id: Option<i64>,
    last: bool,
) -> Result<i64, AppError> {
    if session_id.is_some() && last {
        return Err(AppError::Config(
            "pass either a session id or --last, not both".to_owned(),
        ));
    }
    if let Some(session_id) = session_id {
        return Ok(session_id);
    }
    let sessions = queries
        .list_session_summaries(SessionListRequest {
            offset: 0,
            limit: Some(1),
            include_subagents: false,
            ..Default::default()
        })
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    sessions
        .first()
        .map(|session| session.id)
        .ok_or_else(|| AppError::Config("no sessions found".to_owned()))
}
