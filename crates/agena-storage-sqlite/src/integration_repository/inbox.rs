use super::*;

pub(super) fn decode_event(row: &sea_orm::QueryResult) -> Result<IntegrationEvent> {
    let binding_id: String = row.try_get("", "binding_id").map_err(error)?;
    let payload: String = row.try_get("", "event_json").map_err(error)?;
    Ok(IntegrationEvent {
        binding_id: binding_id.parse().map_err(error)?,
        epoch: row.try_get("", "epoch").map_err(error)?,
        event: serde_json::from_str(&payload).map_err(error)?,
        state: row.try_get("", "state").map_err(error)?,
        received_at_ms: row.try_get("", "received_at_ms").map_err(error)?,
        session_id: row.try_get("", "session_id").map_err(error)?,
        execution_generation: row.try_get("", "execution_generation").map_err(error)?,
        execution_id: row.try_get("", "execution_id").map_err(error)?,
        note: row.try_get("", "note").map_err(error)?,
    })
}

pub(super) async fn insert_event(db: &impl ConnectionTrait, e: &IntegrationEvent) -> Result<bool> {
    let row = db
        .query_one(statement(
            "INSERT INTO agena_integration_inbox
                 (binding_id,event_id,epoch,state,received_at_ms,is_access,note)
             VALUES(?,?,?,?,?,?,?) ON CONFLICT(binding_id,event_id) DO NOTHING RETURNING id",
            vec![
                e.binding_id.to_string().into(),
                e.event.id.clone().into(),
                e.epoch.into(),
                e.state.clone().into(),
                e.received_at_ms.into(),
                // Match SQLite's ASCII case-insensitive LIKE '/access %'.
                i64::from(
                    e.event.text == "/access"
                        || e.event
                            .text
                            .get(..8)
                            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("/access ")),
                )
                .into(),
                e.note.clone().into(),
            ],
        ))
        .await
        .map_err(error)?;
    let Some(row) = row else { return Ok(false) };
    db.execute(statement(
        "INSERT INTO agena_integration_event_payloads(inbox_id,event_json) VALUES(?,?)",
        vec![
            row.try_get::<i64>("", "id").map_err(error)?.into(),
            json(&e.event)?.into(),
        ],
    ))
    .await
    .map_err(error)?;
    Ok(true)
}

pub(super) async fn put_event(db: &impl ConnectionTrait, e: &IntegrationEvent) -> Result<()> {
    db.execute(statement(
        "UPDATE agena_integration_inbox
         SET state=?,session_id=?,execution_generation=?,execution_id=?,note=?
         WHERE binding_id=? AND event_id=?",
        vec![
            e.state.clone().into(),
            e.session_id.into(),
            e.execution_generation.into(),
            e.execution_id.clone().into(),
            e.note.clone().into(),
            e.binding_id.to_string().into(),
            e.event.id.clone().into(),
        ],
    ))
    .await
    .map_err(error)?;
    Ok(())
}

pub(super) async fn reap_stale_events(
    db: &impl ConnectionTrait,
    id: Uuid,
    epoch: i64,
) -> Result<()> {
    // A crash can leave a native execution receipt before finish_event. That
    // execution still needs reconciliation even after the grant changes.
    db.execute(statement(
        "WITH stale AS MATERIALIZED (
            SELECT id,state='processing' AND (
                execution_id IS NOT NULL OR EXISTS(
                    SELECT 1 FROM agena_deliveries d
                    WHERE d.business_key='integration:'||i.binding_id||':'||i.event_id
                )
            ) AS started FROM agena_integration_inbox i
            WHERE binding_id=? AND state IN ('pending','processing') AND epoch!=?
            ORDER BY epoch,queue_priority,received_at_ms,event_id LIMIT 32
         )
         UPDATE agena_integration_inbox AS i
         SET state=CASE WHEN stale.started THEN 'executing' ELSE 'ignored' END,
             note=CASE WHEN stale.started THEN i.note ELSE 'authorization_revoked' END
         FROM stale WHERE i.id=stale.id",
        vec![id.to_string().into(), epoch.into()],
    ))
    .await
    .map_err(error)?;
    Ok(())
}
