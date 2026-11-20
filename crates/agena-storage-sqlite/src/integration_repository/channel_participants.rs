use super::*;

pub(crate) const PARTICIPANTS_TABLE: &str = "CREATE TABLE IF NOT EXISTS agena_channel_participants (binding_id TEXT NOT NULL REFERENCES agena_integration_bindings(id), resource TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('direct','group','broadcast')), actor TEXT NOT NULL, display_name TEXT, first_seen_at_ms INTEGER NOT NULL, last_seen_at_ms INTEGER NOT NULL CHECK(last_seen_at_ms>=first_seen_at_ms), PRIMARY KEY(binding_id,resource,kind,actor)) WITHOUT ROWID";

const PARTICIPANT_PAGE_SIZE: usize = 100;

pub(super) async fn record_interaction(
    db: &impl ConnectionTrait,
    binding: &IntegrationBinding,
    event: &ExternalEvent,
    received_at_ms: i64,
) -> Result<()> {
    let Some(kind) = binding.observed_channel_kind(event, received_at_ms) else {
        return Ok(());
    };
    db.execute(statement(
        "INSERT INTO agena_channel_participants
            (binding_id,resource,kind,actor,display_name,first_seen_at_ms,last_seen_at_ms)
            VALUES (?,?,?,?,?,?,?)
         ON CONFLICT(binding_id,resource,kind,actor) DO UPDATE SET
            display_name=CASE
                WHEN excluded.display_name IS NOT NULL
                    AND (display_name IS NULL OR excluded.last_seen_at_ms>=last_seen_at_ms)
                THEN excluded.display_name ELSE display_name END,
            first_seen_at_ms=MIN(first_seen_at_ms,excluded.first_seen_at_ms),
            last_seen_at_ms=MAX(last_seen_at_ms,excluded.last_seen_at_ms)",
        vec![
            binding.id.to_string().into(),
            event.resource.clone().into(),
            kind.as_str().into(),
            event.actor.clone().into(),
            event.actor_display_name.clone().into(),
            event.occurred_at_ms.into(),
            event.occurred_at_ms.into(),
        ],
    ))
    .await
    .map_err(error)?;
    Ok(())
}

impl SqliteIntegrationRepository {
    pub(super) async fn observed_channel_rooms_read(
        &self,
        id: Uuid,
    ) -> Result<Vec<ObservedChannelRoom>> {
        self.database
            .query_all(statement(
                "SELECT DISTINCT resource,kind FROM agena_channel_participants WHERE binding_id=?",
                vec![id.to_string().into()],
            ))
            .await
            .map_err(error)?
            .into_iter()
            .map(|row| {
                let kind: String = row.try_get("", "kind").map_err(error)?;
                Ok(ObservedChannelRoom {
                    resource: row.try_get("", "resource").map_err(error)?,
                    kind: ConversationKind::parse(&kind)
                        .ok_or_else(|| error("stored channel conversation kind is invalid"))?,
                })
            })
            .collect()
    }

    pub(super) async fn channel_participants_read(
        &self,
        id: Uuid,
        query: ChannelParticipantsQuery,
    ) -> Result<ChannelParticipantPage> {
        if !agena_domain::valid_channel_id(&query.resource)
            || query
                .after
                .as_deref()
                .is_some_and(|actor| !agena_domain::valid_channel_id(actor))
        {
            return Err(Error::Invalid(
                "use an exact conversation ID and a valid participant cursor",
            ));
        }
        let search = query.search.as_deref().unwrap_or_default().trim();
        if search.len() > 1024 || search.chars().any(char::is_control) {
            return Err(Error::Invalid(
                "participant search exceeds the supported length or contains control characters",
            ));
        }
        let mut participants = self
            .database
            .query_all(statement(
                "SELECT actor,display_name,first_seen_at_ms,last_seen_at_ms
                 FROM agena_channel_participants
                 WHERE binding_id=? AND resource=? AND kind=? AND actor>?
                   AND (?='' OR instr(lower(actor),lower(?))>0 OR instr(lower(display_name),lower(?))>0)
                 ORDER BY actor LIMIT ?",
                vec![
                    id.to_string().into(),
                    query.resource.into(),
                    query.kind.as_str().into(),
                    query.after.unwrap_or_default().into(),
                    search.into(),
                    search.into(),
                    search.into(),
                    ((PARTICIPANT_PAGE_SIZE + 1) as i64).into(),
                ],
            ))
            .await
            .map_err(error)?
            .into_iter()
            .map(|row| {
                Ok(ChannelParticipant {
                    actor: row.try_get("", "actor").map_err(error)?,
                    display_name: row.try_get("", "display_name").map_err(error)?,
                    first_seen_at_ms: row.try_get("", "first_seen_at_ms").map_err(error)?,
                    last_seen_at_ms: row.try_get("", "last_seen_at_ms").map_err(error)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let next_after = if participants.len() > PARTICIPANT_PAGE_SIZE {
            participants.pop();
            participants
                .last()
                .map(|participant| participant.actor.clone())
        } else {
            None
        };
        Ok(ChannelParticipantPage {
            participants,
            next_after,
        })
    }
}

/// Rebuild the directory once for databases with a verified inbound history.
/// Stored role denials took precedence over the trigger check. For such rows,
/// a group or broadcast message needs an explicit bot mention for backfill.
pub(crate) async fn backfill_channel_participants(db: &impl ConnectionTrait) -> Result<()> {
    let bindings = db
        .query_all(statement(
            "SELECT binding_json FROM agena_integration_bindings
         WHERE json_extract(binding_json,'$.spec.mode')='channel'",
            vec![],
        ))
        .await
        .map_err(error)?;
    for row in bindings {
        let payload: String = row.try_get("", "binding_json").map_err(error)?;
        let Ok(binding) = serde_json::from_str::<IntegrationBinding>(&payload) else {
            continue;
        };
        if binding.spec.channel_policy.is_none() {
            continue;
        }
        let mut after = String::new();
        loop {
            let rows = db
                .query_all(statement(
                    "SELECT event_id,event_json FROM agena_integration_inbox
                 WHERE binding_id=? AND event_id>? ORDER BY event_id LIMIT 256",
                    vec![binding.id.to_string().into(), after.as_str().into()],
                ))
                .await
                .map_err(error)?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                after = row.try_get("", "event_id").map_err(error)?;
                let payload: String = row.try_get("", "event_json").map_err(error)?;
                let Ok(record) = serde_json::from_str::<IntegrationEvent>(&payload) else {
                    continue;
                };
                if record.event.validate().is_err() {
                    continue;
                }
                match record.note.as_deref() {
                    Some(
                        "message_not_addressed_to_assistant"
                        | "thread_not_authorized"
                        | "conversation_kind_not_authorized",
                    ) => continue,
                    Some("member_not_authorized" | "member_blocked" | "membership_expired")
                        if record.event.channel.as_ref().is_some_and(|channel| {
                            channel.kind.supports_threads() && !channel.addressed_to_bot
                        }) =>
                    {
                        continue;
                    }
                    _ => {}
                }
                record_interaction(db, &binding, &record.event, record.received_at_ms).await?;
            }
        }
    }
    Ok(())
}
