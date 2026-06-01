use super::*;

impl ApiService {
    pub async fn list_sessions(
        &self,
        query: crate::local_api::dto::SessionListQuery,
    ) -> ApiResult<PaginatedResponse<SessionResource>> {
        let limit = normalize_limit(query.pagination.limit());
        let cursor = query
            .pagination
            .cursor()
            .map(decode_cursor::<SessionCursor>)
            .transpose()?;
        let mut statement = entities::session::Entity::find()
            .order_by_desc(entities::session::Column::UpdatedAtMs)
            .order_by_desc(entities::session::Column::Id);

        if let Some(workspace_id) = query.workspace_id {
            statement = statement.filter(entities::session::Column::WorkspaceId.eq(workspace_id));
        }
        if query.roots {
            statement = statement.filter(entities::session::Column::ParentId.is_null());
        }
        if let Some(parent_id) = query.parent_id {
            statement = statement.filter(entities::session::Column::ParentId.eq(parent_id));
        }
        if let Some(search) = non_empty(query.pagination.search()) {
            statement =
                statement.filter(entities::session::Column::Title.like(format!("%{search}%")));
        }
        if let Some(cursor) = cursor {
            statement = statement.filter(
                Condition::any()
                    .add(entities::session::Column::UpdatedAtMs.lt(cursor.updated_at_ms))
                    .add(
                        Condition::all()
                            .add(entities::session::Column::UpdatedAtMs.eq(cursor.updated_at_ms))
                            .add(entities::session::Column::Id.lt(cursor.id)),
                    ),
            );
        }

        let rows = statement
            .limit(limit + 1)
            .all(self.db.as_ref())
            .await
            .map_err(db_error)?;
        let (slice, has_more) = trim_page(rows, limit)?;
        let resources = self.session_resources_from_models(slice.as_slice()).await?;
        let next_cursor = slice.last().map(|row| SessionCursor {
            updated_at_ms: row.updated_at_ms,
            id: row.id,
        });

        build_page(resources, has_more, next_cursor, PageOrder::Desc, limit)
    }

    pub async fn get_session(&self, session_id: i64) -> ApiResult<Option<SessionResource>> {
        let Some(model) = entities::session::Entity::find_by_id(session_id)
            .one(self.db.as_ref())
            .await
            .map_err(db_error)?
        else {
            return Ok(None);
        };

        let mut resources = self.session_resources_from_models(&[model]).await?;
        Ok(resources.pop())
    }

    pub async fn create_session(
        &self,
        request: SessionCreateRequest,
    ) -> ApiResult<SessionResource> {
        self.ensure_workspace_exists(request.workspace_id).await?;
        if let Some(parent_id) = request.session.parent_id {
            let parent = self.ensure_session_model(parent_id).await?;
            if parent.workspace_id != request.workspace_id {
                return Err(ApiError::bad_request(
                    "parent session must belong to the same workspace",
                ));
            }
        }

        let created = session_crud::create_session(
            self.db.as_ref(),
            request.workspace_id,
            request.session.parent_id,
            request.session.title,
        )
        .await
        .map_err(db_error)?;

        let mut resources = self.session_resources_from_models(&[created]).await?;
        resources
            .pop()
            .ok_or_else(|| ApiError::internal("failed to materialize created session"))
    }

    pub async fn replace_session(
        &self,
        session_id: i64,
        request: SessionHierarchyRequest,
    ) -> ApiResult<SessionResource> {
        let existing = self.ensure_session_model(session_id).await?;
        if request.parent_id == Some(session_id) {
            return Err(ApiError::bad_request(
                "session cannot be its own parent session",
            ));
        }
        if let Some(parent_id) = request.parent_id {
            let parent = self.ensure_session_model(parent_id).await?;
            if parent.workspace_id != existing.workspace_id {
                return Err(ApiError::bad_request(
                    "parent session must belong to the same workspace",
                ));
            }
        }

        let next_version = existing.version + 1;
        let mut active: entities::session::ActiveModel = existing.into();
        active.title = Set(request.title);
        active.parent_id = Set(request.parent_id);
        active.version = Set(next_version);
        active.updated_at_ms = Set(Utc::now().timestamp_millis());
        let updated = active.update(self.db.as_ref()).await.map_err(db_error)?;
        let mut resources = self.session_resources_from_models(&[updated]).await?;
        resources
            .pop()
            .ok_or_else(|| ApiError::internal("failed to materialize updated session"))
    }

    pub async fn delete_session(&self, session_id: i64) -> ApiResult<SessionResource> {
        let existing = self.ensure_session_model(session_id).await?;
        let mut resources = self
            .session_resources_from_models(std::slice::from_ref(&existing))
            .await?;
        entities::session::Entity::delete_by_id(session_id)
            .exec(self.db.as_ref())
            .await
            .map_err(db_error)?;
        resources
            .pop()
            .ok_or_else(|| ApiError::internal("failed to materialize deleted session"))
    }

    pub async fn list_session_events(
        &self,
        manager: &SessionManager,
        session_id: i64,
        query: CursorPaginationQuery,
    ) -> ApiResult<PaginatedResponse<agena::event::DomainEvent>> {
        self.ensure_session_exists(session_id).await?;
        let limit = normalize_limit(query.limit);
        let cursor = query
            .cursor
            .as_deref()
            .map(decode_cursor::<EventCursor>)
            .transpose()?;

        let all = manager
            .list_session_events(session_id)
            .await
            .map_err(api_error_from_app)?;

        // Newest-first, then apply cursor + limit, then re-sort ascending so
        // each returned page is stable for clients that append rows in order.
        let mut newest_first: Vec<_> = all.into_iter().collect();
        newest_first.sort_by(|a, b| b.meta.seq_global.cmp(&a.meta.seq_global));
        if let Some(cursor) = cursor {
            newest_first.retain(|e| e.meta.seq_global < cursor.seq);
        }
        let has_more = newest_first.len() > limit as usize;
        let mut slice: Vec<_> = newest_first.into_iter().take(limit as usize).collect();
        let next_cursor = slice.last().map(|e| EventCursor {
            seq: e.meta.seq_global,
            id: e.meta.seq_global,
        });
        slice.reverse();

        build_page(slice, has_more, next_cursor, PageOrder::Asc, limit)
    }
}

impl ApiService {
    async fn session_resources_from_models(
        &self,
        models: &[entities::session::Model],
    ) -> ApiResult<Vec<SessionResource>> {
        if models.is_empty() {
            return Ok(Vec::new());
        }

        let session_ids = models.iter().map(|row| row.id).collect::<Vec<_>>();
        // Per-session message stats are computed from the unified event log
        // by `SessionManager::list_session_events`. The HTTP service no
        // longer queries a per-session stats table.
        let mut message_stats: HashMap<i64, session_crud::SessionMessageStats> = HashMap::new();
        // Without access to the SessionManager here we conservatively
        // report zero counts. Routes that want accurate per-session counts
        // should fetch them via the dedicated stats endpoint or compute
        // them client-side.
        for &id in &session_ids {
            let _ = id;
        }
        let _ = (&mut message_stats,);
        let child_counts =
            session_crud::child_session_counts_by_parent_ids(self.db.as_ref(), &session_ids)
                .await
                .map_err(db_error)?;

        models
            .iter()
            .map(|model| session_resource(model, &message_stats, &child_counts))
            .collect()
    }
}

fn session_resource(
    model: &entities::session::Model,
    message_stats: &HashMap<i64, session_crud::SessionMessageStats>,
    child_counts: &HashMap<i64, i64>,
) -> ApiResult<SessionResource> {
    let stats = message_stats.get(&model.id).copied();
    let message_count = stats
        .map(|item| u64::try_from(item.message_count))
        .transpose()
        .map_err(|_| {
            ApiError::internal(format!(
                "invalid negative message count for session {}",
                model.id
            ))
        })?
        .unwrap_or_default();
    let child_session_count = child_counts
        .get(&model.id)
        .copied()
        .map(u64::try_from)
        .transpose()
        .map_err(|_| {
            ApiError::internal(format!(
                "invalid negative child session count for session {}",
                model.id
            ))
        })?
        .unwrap_or_default();

    Ok(SessionResource {
        id: model.id,
        parent_id: model.parent_id,
        depth: model.depth,
        root_id: model.root_id,
        workspace_id: model.workspace_id,
        title: model.title.clone(),
        version: model.version,
        is_subagent: model.is_subagent,
        created_at: timestamp_millis_to_utc(model.created_at_ms)?,
        updated_at: timestamp_millis_to_utc(model.updated_at_ms)?,
        message_count,
        child_session_count,
        last_message_at: stats
            .and_then(|item| item.last_message_at_ms)
            .map(timestamp_millis_to_utc)
            .transpose()?,
    })
}
