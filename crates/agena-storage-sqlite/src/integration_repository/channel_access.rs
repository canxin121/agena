use super::*;

pub(crate) const DECLARATIONS: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_agena_channel_inbox_execution ON agena_integration_inbox(binding_id,json_extract(event_json,'$.execution_id'))",
    "CREATE TABLE IF NOT EXISTS agena_channel_sessions (session_id INTEGER PRIMARY KEY, binding_id TEXT NOT NULL REFERENCES agena_integration_bindings(id), scope_key TEXT NOT NULL, epoch INTEGER NOT NULL, actor TEXT NOT NULL, record_json TEXT NOT NULL CHECK(json_valid(record_json)), UNIQUE(binding_id,scope_key))",
    "CREATE INDEX IF NOT EXISTS idx_agena_channel_sessions_binding ON agena_channel_sessions(binding_id,epoch,session_id)",
    "CREATE TABLE IF NOT EXISTS agena_channel_access_audit (binding_id TEXT NOT NULL REFERENCES agena_integration_bindings(id), request_key TEXT NOT NULL, input_json TEXT NOT NULL CHECK(json_valid(input_json)), result_json TEXT NOT NULL CHECK(json_valid(result_json)), audit_json TEXT NOT NULL CHECK(json_valid(audit_json)), at_ms INTEGER NOT NULL, PRIMARY KEY(binding_id,request_key)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_channel_user_budget (binding_id TEXT NOT NULL REFERENCES agena_integration_bindings(id), actor TEXT NOT NULL, utc_day TEXT NOT NULL, used INTEGER NOT NULL CHECK(used>=0), PRIMARY KEY(binding_id,actor,utc_day)) WITHOUT ROWID",
];

impl SqliteIntegrationRepository {
    pub(super) async fn channel_policy_write(
        &self,
        id: Uuid,
        r: ChannelPolicyUpdate,
        actor: &str,
        now: i64,
    ) -> Result<IntegrationBinding> {
        if r.request_key.is_empty() || r.request_key.len() > 160 || actor.len() > 512 {
            return Err(Error::Invalid("invalid access change identity"));
        }
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(error)?;
        let tx = self.database.begin().await.map_err(error)?;
        let input = json(&serde_json::json!({"request":r,"actor":actor}))?;
        if let Some(row)=tx.query_one(statement("SELECT input_json,result_json FROM agena_channel_access_audit WHERE binding_id=? AND request_key=?",vec![id.to_string().into(),r.request_key.clone().into()])).await.map_err(error)? {
            if row.try_get::<String>("","input_json").map_err(error)?!=input{return Err(Error::Conflict("access request identity was reused with different input"));}
            return decode(row,"result_json");
        }
        // Replaying a committed receipt never restores access; it stays valid
        // even if that recorded owner's assignment has since expired.
        r.policy.validate(now).map_err(Error::Policy)?;
        let mut b = read_binding(&tx, id).await?;
        if b.spec.mode != IntegrationMode::Channel {
            return Err(Error::Invalid(
                "only channel bindings have member access policy",
            ));
        }
        if b.revision != r.expected_revision {
            return Err(Error::Conflict(
                "channel permissions changed; reload and explicitly reconfirm",
            ));
        }
        let before = b.spec.channel_policy.clone();
        if before
            .as_ref()
            .is_some_and(|p| p.identity != r.policy.identity)
        {
            return Err(Error::Conflict(
                "tenant/account/conversation kind cannot be changed on an existing binding; create a new connection",
            ));
        }
        b.spec.channel_policy = Some(r.policy.clone());
        b.spec.actors.clear();
        b.revision += 1;
        b.epoch += 1;
        b.updated_at_ms = now;
        b.next_poll_ms = now;
        b.last_error = None;
        // New permissions get new scoped sessions. A prior privileged context
        // is never reused by a newly added or downgraded member.
        b.channel_session_id = None;
        put_binding(&tx, &b, r.expected_revision).await?;
        let audit = ChannelAccessAudit {
            binding_id: id,
            request_key: r.request_key.clone(),
            actor: actor.into(),
            revision: b.revision,
            epoch: b.epoch,
            at_ms: now,
            before,
            after: r.policy,
        };
        tx.execute(statement("INSERT INTO agena_channel_access_audit(binding_id,request_key,input_json,result_json,audit_json,at_ms) VALUES(?,?,?,?,?,?)",vec![id.to_string().into(),r.request_key.into(),input.into(),json(&b)?.into(),json(&audit)?.into(),now.into()])).await.map_err(error)?;
        tx.commit().await.map_err(error)?;
        Ok(b)
    }
    pub(super) async fn channel_audit_read(&self, id: Uuid) -> Result<Vec<ChannelAccessAudit>> {
        self.database.query_all(statement("SELECT audit_json FROM agena_channel_access_audit WHERE binding_id=? ORDER BY at_ms DESC,request_key LIMIT 100",vec![id.to_string().into()])).await.map_err(error)?.into_iter().map(|r|decode(r,"audit_json")).collect()
    }
    pub(super) async fn channel_sessions_read(
        &self,
        id: Uuid,
    ) -> Result<Vec<ChannelSessionRecord>> {
        self.database.query_all(statement("SELECT record_json FROM agena_channel_sessions WHERE binding_id=? ORDER BY epoch DESC,session_id DESC LIMIT 2048",vec![id.to_string().into()])).await.map_err(error)?.into_iter().map(|r|decode(r,"record_json")).collect()
    }
    pub(super) async fn channel_session_read(
        &self,
        id: Uuid,
        key: &str,
    ) -> Result<Option<ChannelSessionRecord>> {
        self.database
            .query_one(statement(
                "SELECT record_json FROM agena_channel_sessions WHERE binding_id=? AND scope_key=?",
                vec![id.to_string().into(), key.into()],
            ))
            .await
            .map_err(error)?
            .map(|r| decode(r, "record_json"))
            .transpose()
    }
    /// Drop the scope record for a room whose conversation no longer exists.
    ///
    /// Deleting a conversation soft-deletes the session row; the scope record
    /// would otherwise keep resolving that room to a dead session and every
    /// later message would fail. Removing the row lets the next message create
    /// a fresh conversation under the same binding and authority.
    pub(super) async fn channel_session_forget(&self, id: Uuid, key: &str) -> Result<()> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(error)?;
        let tx = self.database.begin().await.map_err(error)?;
        // Keep the memory directory layout stable: it is keyed by the scope
        // hash, not by the session row, so a fresh conversation reuses it.
        tx.execute(statement(
            "DELETE FROM agena_channel_sessions WHERE binding_id=? AND scope_key=?",
            vec![id.to_string().into(), key.into()],
        ))
        .await
        .map_err(error)?;
        tx.commit().await.map_err(error)?;
        Ok(())
    }
    pub(super) async fn channel_session_write(
        &self,
        record: ChannelSessionRecord,
    ) -> Result<ChannelSessionRecord> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(error)?;
        let tx = self.database.begin().await.map_err(error)?;
        let b = read_binding(&tx, record.binding_id).await?;
        if !b.authorizes_scope(&record.authority, record.created_at_ms) {
            return Err(Error::Conflict(
                "channel authority changed before session admission",
            ));
        }
        if let Some(row) = tx
            .query_one(statement(
                "SELECT record_json FROM agena_channel_sessions WHERE binding_id=? AND scope_key=?",
                vec![
                    record.binding_id.to_string().into(),
                    record.scope_key.clone().into(),
                ],
            ))
            .await
            .map_err(error)?
        {
            let old: ChannelSessionRecord = decode(row, "record_json")?;
            if old.session_id != record.session_id || old.authority != record.authority {
                return Err(Error::Conflict(
                    "channel scope key already belongs to different authority",
                ));
            }
            return Ok(old);
        }
        let count = tx
            .query_one(statement(
                "SELECT COUNT(*) AS n FROM agena_channel_sessions WHERE binding_id=?",
                vec![b.id.to_string().into()],
            ))
            .await
            .map_err(error)?
            .ok_or(Error::NotFound)?
            .try_get::<i64>("", "n")
            .map_err(error)?;
        if count >= 2048 {
            return Err(Error::Conflict(
                "channel session capacity reached; archive the binding before accepting more contexts",
            ));
        }
        tx.execute(statement("INSERT INTO agena_channel_sessions(session_id,binding_id,scope_key,epoch,actor,record_json) VALUES(?,?,?,?,?,?)",vec![record.session_id.into(),record.binding_id.to_string().into(),record.scope_key.clone().into(),record.authority.epoch.into(),record.authority.actor.clone().into(),json(&record)?.into()])).await.map_err(error)?;
        tx.commit().await.map_err(error)?;
        Ok(record)
    }
}

pub(super) async fn charge_channel_user(
    db: &impl ConnectionTrait,
    b: &IntegrationBinding,
    event: &ExternalEvent,
    day: &str,
) -> Result<bool> {
    let Some(policy) = b
        .spec
        .channel_policy
        .as_ref()
        .filter(|_| b.spec.mode == IntegrationMode::Channel)
    else {
        return Ok(true);
    };
    let count=db.query_one(statement("SELECT used FROM agena_channel_user_budget WHERE binding_id=? AND actor=? AND utc_day=?",vec![b.id.to_string().into(),event.actor.clone().into(),day.into()])).await.map_err(error)?.map(|r|r.try_get::<i64>("","used").map_err(error)).transpose()?.unwrap_or(0);
    if count >= i64::from(policy.per_user_daily_limit) {
        return Ok(false);
    }
    db.execute(statement("INSERT INTO agena_channel_user_budget(binding_id,actor,utc_day,used) VALUES(?,?,?,1) ON CONFLICT(binding_id,actor,utc_day) DO UPDATE SET used=used+1",vec![b.id.to_string().into(),event.actor.clone().into(),day.into()])).await.map_err(error)?;
    Ok(true)
}

/// Process authenticated loss-of-membership before ordinary inbox admission.
/// This path can only remove authority, never infer a new role from platform
/// metadata. It is not charged against chat or execution quotas.
pub(super) async fn ingest_platform_revocations(
    db: &impl ConnectionTrait,
    b: &mut IntegrationBinding,
    events: &[ExternalEvent],
    now: i64,
) -> Result<()> {
    if b.spec.mode != IntegrationMode::Channel {
        return Ok(());
    }
    for event in events
        .iter()
        .filter(|event| event.event_type == "membership.revoked")
    {
        if db
            .query_one(statement(
                "SELECT event_id FROM agena_integration_inbox WHERE binding_id=? AND event_id=?",
                vec![b.id.to_string().into(), event.id.clone().into()],
            ))
            .await
            .map_err(error)?
            .is_some()
        {
            continue;
        }
        let valid = b
            .spec
            .channel_policy
            .as_ref()
            .zip(event.channel.as_ref())
            .is_some_and(|(policy, context)| {
                agena_domain::valid_channel_id(&event.actor)
                    && context.identity == policy.identity
                    && b.spec.resource.matches(&event.resource)
                    && event.occurred_at_ms >= b.activated_at_ms.unwrap_or(b.created_at_ms)
                    && event.occurred_at_ms <= now + 30000
                    && context.actor_verified
                    && !context.is_member
                    && context.verified_at_ms >= now - 300000
                    && context.verified_at_ms <= now + 30000
            });
        let mut note = "invalid_platform_revocation".to_owned();
        if valid {
            let before = b
                .spec
                .channel_policy
                .clone()
                .ok_or(Error::Invalid("missing channel policy"))?;
            let mut next = before.clone();
            let changed = if let Some(member) =
                next.members.iter_mut().find(|m| m.actor == event.actor)
            {
                if member.role == ChannelRole::Blocked {
                    false
                } else {
                    member.role = ChannelRole::Blocked;
                    true
                }
            } else if next.members.len() < 128 {
                next.members.push(ChannelMember {
                    actor: event.actor.clone(),
                    role: ChannelRole::Blocked,
                    expires_at_ms: None,
                });
                true
            } else {
                b.state = BindingState::Faulted;
                b.last_error=Some("platform revocation could not fit in the member policy; connection stopped for operator review".into());
                true
            };
            note = "verified_platform_revocation".into();
            if changed {
                if next.validate(now).is_err() {
                    b.state = BindingState::Faulted;
                    b.last_error=Some("platform revoked membership and the policy has no valid owner; instance operator review is required".into());
                }
                b.spec.channel_policy = Some(next.clone());
                b.epoch += 1;
                b.revision += 1;
                b.channel_session_id = None;
                b.updated_at_ms = now;
                let audit = ChannelAccessAudit {
                    binding_id: b.id,
                    request_key: format!("platform:{}", event.id),
                    actor: format!("platform:{}", b.spec.provider),
                    revision: b.revision,
                    epoch: b.epoch,
                    at_ms: now,
                    before: Some(before),
                    after: next,
                };
                db.execute(statement("INSERT INTO agena_channel_access_audit(binding_id,request_key,input_json,result_json,audit_json,at_ms) VALUES(?,?,?,?,?,?)",vec![b.id.to_string().into(),audit.request_key.clone().into(),json(event)?.into(),json(b)?.into(),json(&audit)?.into(),now.into()])).await.map_err(error)?;
            }
        }
        let record = IntegrationEvent {
            binding_id: b.id,
            epoch: b.epoch,
            event: event.clone(),
            state: if valid { "handled" } else { "ignored" }.into(),
            received_at_ms: now,
            session_id: None,
            execution_generation: None,
            execution_id: None,
            note: Some(note),
        };
        db.execute(statement("INSERT INTO agena_integration_inbox(binding_id,event_id,state,received_at_ms,event_json) VALUES(?,?,?,?,?)",vec![b.id.to_string().into(),event.id.clone().into(),record.state.clone().into(),now.into(),json(&record)?.into()])).await.map_err(error)?;
    }
    Ok(())
}
