use super::*;

pub(crate) const DECLARATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_channel_rooms (id INTEGER PRIMARY KEY, binding_id TEXT NOT NULL REFERENCES agena_integration_bindings(id), resource TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('direct','group','broadcast')), UNIQUE(binding_id,resource,kind))",
    "CREATE TABLE IF NOT EXISTS agena_channel_participants (room_id INTEGER NOT NULL REFERENCES agena_channel_rooms(id), actor TEXT NOT NULL, display_name TEXT, first_seen_at_ms INTEGER NOT NULL, last_seen_at_ms INTEGER NOT NULL CHECK(last_seen_at_ms>=first_seen_at_ms), PRIMARY KEY(room_id,actor)) WITHOUT ROWID",
];

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
    let room = [
        binding.id.to_string().into(),
        event.resource.clone().into(),
        kind.as_str().into(),
    ];
    db.execute(statement(
        "INSERT INTO agena_channel_rooms(binding_id,resource,kind) VALUES (?,?,?)
         ON CONFLICT(binding_id,resource,kind) DO NOTHING",
        room.to_vec(),
    ))
    .await
    .map_err(error)?;
    let mut args = vec![
        event.actor.clone().into(),
        event.actor_display_name.clone().into(),
        event.occurred_at_ms.into(),
        event.occurred_at_ms.into(),
    ];
    args.extend(room);
    db.execute(statement(
        "INSERT INTO agena_channel_participants
            (room_id,actor,display_name,first_seen_at_ms,last_seen_at_ms)
         SELECT id,?,?,?,? FROM agena_channel_rooms WHERE binding_id=? AND resource=? AND kind=?
         ON CONFLICT(room_id,actor) DO UPDATE SET
            display_name=CASE
                WHEN excluded.display_name IS NOT NULL
                    AND (display_name IS NULL OR excluded.last_seen_at_ms>=last_seen_at_ms)
                THEN excluded.display_name ELSE display_name END,
            first_seen_at_ms=MIN(first_seen_at_ms,excluded.first_seen_at_ms),
            last_seen_at_ms=MAX(last_seen_at_ms,excluded.last_seen_at_ms)",
        args,
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
        self.reader
            .query_all(statement(
                "SELECT resource,kind FROM agena_channel_rooms WHERE binding_id=? ORDER BY resource,kind",
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
            .reader
            .query_all(statement(
                "SELECT actor,display_name,first_seen_at_ms,last_seen_at_ms
                 FROM agena_channel_participants
                 WHERE room_id=(SELECT id FROM agena_channel_rooms WHERE binding_id=? AND resource=? AND kind=?)
                   AND actor>?
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
