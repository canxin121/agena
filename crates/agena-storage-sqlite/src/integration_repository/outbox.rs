use super::*;

pub(super) fn decode_delivery(
    row: &sea_orm::QueryResult,
    text: String,
) -> Result<IntegrationDelivery> {
    let binding_id: String = row.try_get("", "binding_id").map_err(error)?;
    let route: Option<String> = row.try_get("", "route_json").map_err(error)?;
    let attempts: i64 = row.try_get("", "attempts").map_err(error)?;
    Ok(IntegrationDelivery {
        id: row.try_get("", "id").map_err(error)?,
        binding_id: binding_id.parse().map_err(error)?,
        text,
        state: row.try_get("", "state").map_err(error)?,
        attempts: attempts.try_into().map_err(error)?,
        next_attempt_ms: row.try_get("", "next_attempt_ms").map_err(error)?,
        remote_id: row.try_get("", "remote_id").map_err(error)?,
        created_at_ms: row.try_get("", "created_at_ms").map_err(error)?,
        error: row.try_get("", "error").map_err(error)?,
        route: route
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(error)?,
    })
}

pub(super) async fn read_delivery_text(
    db: &impl ConnectionTrait,
    delivery_key: i64,
) -> Result<String> {
    db.query_one(statement(
        "SELECT text FROM agena_integration_delivery_text WHERE delivery_key=?",
        vec![delivery_key.into()],
    ))
    .await
    .map_err(error)?
    .map(|row| row.try_get("", "text").map_err(error))
    .transpose()
    .map(Option::unwrap_or_default)
}

pub(super) async fn put_delivery(db: &impl ConnectionTrait, d: &IntegrationDelivery) -> Result<()> {
    db.execute(statement(
        "UPDATE agena_integration_outbox SET state=?,attempts=?,next_attempt_ms=?,remote_id=?,error=?
         WHERE id=? AND binding_id=?",
        vec![
            d.state.clone().into(),
            i64::from(d.attempts).into(),
            d.next_attempt_ms.into(),
            d.remote_id.clone().into(),
            d.error.clone().into(),
            d.id.clone().into(),
            d.binding_id.to_string().into(),
        ],
    ))
    .await
    .map_err(error)?;
    Ok(())
}

pub(super) async fn insert_delivery(
    db: &DatabaseConnection,
    d: IntegrationDelivery,
    require_pending_execution: bool,
) -> Result<()> {
    if d.text.len() > 24576 || d.id.len() > 160 {
        return Err(Error::Invalid("channel delivery exceeds bounded contract"));
    }
    let route = d.route.as_ref().map(json).transpose()?;
    let _permit = crate::acquire_write_permit(db).await.map_err(error)?;
    let tx = db.begin().await.map_err(error)?;
    let row = tx
        .query_one(statement(
            "INSERT INTO agena_integration_outbox
             (id,binding_id,state,attempts,next_attempt_ms,created_at_ms,remote_id,error,route_json)
         SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9
         WHERE ?10=0 OR EXISTS(
             SELECT 1 FROM agena_channel_pending_outputs p
             JOIN agena_channel_sessions c ON c.session_id=p.session_id
             WHERE p.delivery_id=?1 AND c.binding_id=?2)
         ON CONFLICT(id) DO NOTHING RETURNING delivery_key",
            vec![
                d.id.into(),
                d.binding_id.to_string().into(),
                d.state.into(),
                i64::from(d.attempts).into(),
                d.next_attempt_ms.into(),
                d.created_at_ms.into(),
                d.remote_id.into(),
                d.error.into(),
                route.into(),
                i64::from(require_pending_execution).into(),
            ],
        ))
        .await
        .map_err(error)?;
    if let Some(row) = row.filter(|_| !d.text.is_empty()) {
        tx.execute(statement(
            "INSERT INTO agena_integration_delivery_text(delivery_key,text) VALUES(?,?)",
            vec![
                row.try_get::<i64>("", "delivery_key")
                    .map_err(error)?
                    .into(),
                d.text.into(),
            ],
        ))
        .await
        .map_err(error)?;
    }
    tx.commit().await.map_err(error)?;
    Ok(())
}
