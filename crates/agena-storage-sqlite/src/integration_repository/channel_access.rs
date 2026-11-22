use super::*;

pub(crate) const DECLARATIONS: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_agena_channel_inbox_execution ON agena_integration_inbox(binding_id,execution_id) WHERE execution_id IS NOT NULL",
    "CREATE TABLE IF NOT EXISTS agena_channel_sessions (session_id INTEGER PRIMARY KEY, binding_id TEXT NOT NULL REFERENCES agena_integration_bindings(id), scope_key TEXT NOT NULL, epoch INTEGER NOT NULL, has_work INTEGER NOT NULL DEFAULT 0 CHECK(has_work IN (0,1)), record_json TEXT NOT NULL CHECK(json_valid(record_json)), UNIQUE(binding_id,scope_key))",
    "CREATE INDEX IF NOT EXISTS idx_agena_channel_sessions_binding ON agena_channel_sessions(binding_id,epoch,session_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_channel_sessions_work ON agena_channel_sessions(binding_id,epoch,session_id) WHERE has_work=1",
    "CREATE TABLE IF NOT EXISTS agena_channel_pending_outputs (
        session_id INTEGER NOT NULL REFERENCES agena_channel_sessions(session_id) ON DELETE CASCADE,
        finished_at_ms INTEGER NOT NULL,
        execution_id TEXT NOT NULL,
        generation INTEGER NOT NULL,
        delivery_id TEXT NOT NULL UNIQUE,
        PRIMARY KEY(session_id,finished_at_ms,execution_id,generation),
        FOREIGN KEY(execution_id,generation,session_id) REFERENCES agena_attempts(execution_id,generation,session_id) ON DELETE CASCADE
     ) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_channel_access_audit (binding_id TEXT NOT NULL REFERENCES agena_integration_bindings(id), request_key TEXT NOT NULL, input_json TEXT NOT NULL CHECK(json_valid(input_json)), result_json BLOB NOT NULL CHECK(typeof(result_json)='blob' AND length(result_json)>1), audit_json TEXT NOT NULL CHECK(json_valid(audit_json)), at_ms INTEGER NOT NULL, PRIMARY KEY(binding_id,request_key)) WITHOUT ROWID",
    "CREATE INDEX IF NOT EXISTS idx_agena_channel_access_recent ON agena_channel_access_audit(binding_id,at_ms DESC,request_key)",
    "CREATE TABLE IF NOT EXISTS agena_channel_user_budget (binding_id TEXT NOT NULL REFERENCES agena_integration_bindings(id), actor TEXT NOT NULL, utc_day TEXT NOT NULL, used INTEGER NOT NULL CHECK(used>=0), PRIMARY KEY(binding_id,actor,utc_day)) WITHOUT ROWID",
    "CREATE VIEW IF NOT EXISTS agena_channel_session_work AS
     SELECT c.session_id,CASE
        WHEN EXISTS(SELECT 1 FROM agena_channel_pending_outputs p WHERE p.session_id=c.session_id) THEN 1
        ELSE EXISTS(
            SELECT 1 FROM agena_interactions i WHERE i.session_id=c.session_id AND i.decision_json IS NULL
              AND NOT EXISTS(SELECT 1 FROM agena_integration_outbox o WHERE o.id=c.binding_id||':attention:'||i.id)
        ) END AS has_work
     FROM agena_channel_sessions c",
    "CREATE TRIGGER IF NOT EXISTS agena_channel_ready_output_insert AFTER INSERT ON agena_channel_pending_outputs BEGIN
        UPDATE agena_channel_sessions SET has_work=1 WHERE session_id=NEW.session_id AND has_work=0;
     END",
    "CREATE TRIGGER IF NOT EXISTS agena_channel_ready_output_delete AFTER DELETE ON agena_channel_pending_outputs BEGIN
        UPDATE agena_channel_sessions SET has_work=(
            SELECT has_work FROM agena_channel_session_work WHERE session_id=OLD.session_id
        ) WHERE session_id=OLD.session_id;
     END",
    "CREATE TRIGGER IF NOT EXISTS agena_channel_ready_attention_insert AFTER INSERT ON agena_interactions
     WHEN NEW.decision_json IS NULL BEGIN
        UPDATE agena_channel_sessions SET has_work=1 WHERE session_id=NEW.session_id AND has_work=0
          AND NOT EXISTS(SELECT 1 FROM agena_integration_outbox WHERE id=agena_channel_sessions.binding_id||':attention:'||NEW.id);
     END",
    "CREATE TRIGGER IF NOT EXISTS agena_channel_ready_attention_decided AFTER UPDATE OF decision_json ON agena_interactions
     WHEN (OLD.decision_json IS NULL)!=(NEW.decision_json IS NULL) BEGIN
        UPDATE agena_channel_sessions SET has_work=(
            SELECT has_work FROM agena_channel_session_work WHERE session_id=NEW.session_id
        ) WHERE session_id=NEW.session_id;
     END",
    "CREATE TRIGGER IF NOT EXISTS agena_channel_ready_attention_delete AFTER DELETE ON agena_interactions
     WHEN OLD.decision_json IS NULL BEGIN
        UPDATE agena_channel_sessions SET has_work=(
            SELECT has_work FROM agena_channel_session_work WHERE session_id=OLD.session_id
        ) WHERE session_id=OLD.session_id AND has_work=1;
     END",
    "CREATE TRIGGER IF NOT EXISTS agena_channel_ready_attention_enqueued AFTER INSERT ON agena_integration_outbox
     WHEN NEW.id GLOB '*:attention:*' BEGIN
        UPDATE agena_channel_sessions SET has_work=(
            SELECT has_work FROM agena_channel_session_work
            WHERE session_id=agena_channel_sessions.session_id
        ) WHERE session_id=(
            SELECT i.session_id FROM agena_interactions i
            JOIN agena_channel_sessions c ON c.session_id=i.session_id
            WHERE i.id=CAST(substr(NEW.id,instr(NEW.id,':attention:')+length(':attention:')) AS INTEGER)
              AND i.decision_json IS NULL AND NEW.id=c.binding_id||':attention:'||i.id
        ) AND has_work=1;
     END",
    "CREATE TRIGGER IF NOT EXISTS agena_channel_ready_attention_reopened AFTER DELETE ON agena_integration_outbox
     WHEN OLD.id GLOB '*:attention:*' BEGIN
        UPDATE agena_channel_sessions SET has_work=1 WHERE session_id=(
            SELECT i.session_id FROM agena_interactions i
            JOIN agena_channel_sessions c ON c.session_id=i.session_id
            WHERE i.id=CAST(substr(OLD.id,instr(OLD.id,':attention:')+length(':attention:')) AS INTEGER)
              AND i.decision_json IS NULL AND OLD.id=c.binding_id||':attention:'||i.id
        ) AND has_work=0;
     END",
    // Completion, generation changes and outbox acknowledgements share the
    // transaction of the durable fact that changes the export queue.
    "CREATE TRIGGER IF NOT EXISTS agena_channel_output_finished AFTER UPDATE OF phase ON agena_attempts
     WHEN OLD.finished_at_ms IS NULL AND NEW.finished_at_ms IS NOT NULL BEGIN
        INSERT INTO agena_channel_pending_outputs(session_id,finished_at_ms,execution_id,generation,delivery_id)
        SELECT c.session_id,NEW.finished_at_ms,NEW.execution_id,NEW.generation,c.binding_id||':'||NEW.execution_id||':'||NEW.generation
        FROM agena_channel_sessions c
        CROSS JOIN agena_executions e ON e.id=NEW.execution_id AND e.current_generation=NEW.generation
        WHERE c.session_id=NEW.session_id AND NOT EXISTS(
            SELECT 1 FROM agena_integration_outbox o WHERE o.id=c.binding_id||':'||NEW.execution_id||':'||NEW.generation
        );
     END",
    "CREATE TRIGGER IF NOT EXISTS agena_channel_output_attached AFTER INSERT ON agena_channel_sessions BEGIN
        INSERT INTO agena_channel_pending_outputs(session_id,finished_at_ms,execution_id,generation,delivery_id)
        SELECT NEW.session_id,a.finished_at_ms,e.id,a.generation,NEW.binding_id||':'||e.id||':'||a.generation
        FROM agena_executions e
        CROSS JOIN agena_attempts a ON a.execution_id=e.id AND a.generation=e.current_generation
        WHERE e.session_id=NEW.session_id AND a.finished_at_ms IS NOT NULL AND NOT EXISTS(
            SELECT 1 FROM agena_integration_outbox o WHERE o.id=NEW.binding_id||':'||e.id||':'||a.generation
        );
        UPDATE agena_channel_sessions SET has_work=(
            SELECT has_work FROM agena_channel_session_work WHERE session_id=NEW.session_id
        ) WHERE session_id=NEW.session_id AND has_work=0;
     END",
    "CREATE TRIGGER IF NOT EXISTS agena_channel_output_retried AFTER UPDATE OF current_generation ON agena_executions
     WHEN OLD.current_generation!=NEW.current_generation BEGIN
        DELETE FROM agena_channel_pending_outputs WHERE delivery_id=(
            SELECT binding_id||':'||OLD.id||':'||OLD.current_generation
            FROM agena_channel_sessions WHERE session_id=OLD.session_id
        );
     END",
    "CREATE TRIGGER IF NOT EXISTS agena_channel_output_enqueued AFTER INSERT ON agena_integration_outbox BEGIN
        DELETE FROM agena_channel_pending_outputs WHERE delivery_id=NEW.id;
     END",
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
        let mut b = read_binding(&tx, id).await?;
        if b.spec.is_telegram_channel() {
            return Err(Error::Invalid(
                "Telegram chat permissions are managed in Permission assignments",
            ));
        }
        let input = json(&serde_json::json!({"request":r,"actor":actor}))?;
        if let Some(row)=tx.query_one(statement("SELECT input_json,result_json FROM agena_channel_access_audit WHERE binding_id=? AND request_key=?",vec![id.to_string().into(),r.request_key.clone().into()])).await.map_err(error)? {
            if row.try_get::<String>("","input_json").map_err(error)?!=input{return Err(Error::Conflict("access request identity was reused with different input"));}
            let result: Vec<u8> = row.try_get("", "result_json").map_err(error)?;
            let result = crate::receipt_payload::unpack(&result).map_err(error)?;
            return serde_json::from_slice(&result).map_err(error);
        }
        // Replaying a committed receipt never restores access; it stays valid
        // even if that recorded owner's assignment has since expired.
        r.policy.validate(now).map_err(Error::Policy)?;
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
        tx.execute(statement("INSERT INTO agena_channel_access_audit(binding_id,request_key,input_json,result_json,audit_json,at_ms) VALUES(?,?,?,?,?,?)",vec![id.to_string().into(),r.request_key.into(),input.into(),pack_receipt(&b)?.into(),json(&audit)?.into(),now.into()])).await.map_err(error)?;
        tx.commit().await.map_err(error)?;
        Ok(b)
    }
    pub(super) async fn channel_review_write(
        &self,
        binding: IntegrationBinding,
        expected_revision: i64,
        actor: &str,
        now: i64,
    ) -> Result<IntegrationBinding> {
        if actor != "instance_operator" {
            return Err(Error::Invalid(
                "channel history review requires the local operator",
            ));
        }
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(error)?;
        let tx = self.database.begin().await.map_err(error)?;
        let previous = read_binding(&tx, binding.id).await?;
        if previous.revision != expected_revision
            || binding.revision != expected_revision + 1
            || binding.epoch != previous.epoch + 1
            || binding.state != BindingState::Enabled
            || binding.spec.mode != IntegrationMode::Channel
            || serde_json::to_value(&binding.spec).map_err(error)?
                != serde_json::to_value(&previous.spec).map_err(error)?
            || binding.source_session_id != previous.source_session_id
            || binding.owner_bot_id != previous.owner_bot_id
            || binding.workspace_id != previous.workspace_id
            || binding.permission_fingerprint != previous.permission_fingerprint
        {
            return Err(Error::Conflict(
                "channel review no longer matches the inspected boundary",
            ));
        }
        let policy = binding
            .spec
            .channel_policy
            .clone()
            .ok_or(Error::Invalid("channel policy required"))?;
        policy.validate(now).map_err(Error::Policy)?;
        let request_key = format!("review:{expected_revision}");
        let audit = ChannelAccessAudit {
            binding_id: binding.id,
            request_key: request_key.clone(),
            actor: actor.into(),
            revision: binding.revision,
            epoch: binding.epoch,
            at_ms: now,
            before: previous.spec.channel_policy,
            after: policy,
        };
        let input = json(
            &serde_json::json!({"operation":"review","expected_revision":expected_revision,"provider_fingerprint":binding.provider_fingerprint,"permission_fingerprint":binding.permission_fingerprint}),
        )?;
        put_binding(&tx, &binding, expected_revision).await?;
        tx.execute(statement("INSERT INTO agena_channel_access_audit(binding_id,request_key,input_json,result_json,audit_json,at_ms) VALUES(?,?,?,?,?,?)",
            vec![binding.id.to_string().into(), request_key.into(), input.into(), pack_receipt(&binding)?.into(), json(&audit)?.into(), now.into()])).await.map_err(error)?;
        tx.commit().await.map_err(error)?;
        Ok(binding)
    }

    pub(super) async fn channel_session_recover(
        &self,
        expected: ChannelSessionRecord,
        current: ChannelSessionRecord,
        now: i64,
    ) -> Result<ChannelSessionRecord> {
        if expected.binding_id != current.binding_id
            || expected.session_id != current.session_id
            || expected.created_at_ms != current.created_at_ms
            || current.scope_key.is_empty()
        {
            return Err(Error::Invalid(
                "channel recovery must preserve its session and binding",
            ));
        }
        let mut immutable = expected.authority.clone();
        immutable.epoch = current.authority.epoch;
        immutable.role = current.authority.role;
        immutable.revoked = current.authority.revoked;
        if immutable != current.authority
            || current.authority.epoch <= expected.authority.epoch
            || current.authority.revoked
            || !expected.authority.active_at(now)
        {
            return Err(Error::Conflict(
                "channel recovery cannot change an existing conversation capability or expiry",
            ));
        }
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(error)?;
        let tx = self.database.begin().await.map_err(error)?;
        let binding = read_binding(&tx, current.binding_id).await?;
        if !binding.authorizes_scope(&current.authority, now) {
            return Err(Error::Conflict(
                "reviewed channel authority is no longer active",
            ));
        }
        let row = tx
            .query_one(statement(
                "SELECT record_json FROM agena_channel_sessions WHERE session_id=?",
                vec![current.session_id.into()],
            ))
            .await
            .map_err(error)?
            .ok_or(Error::NotFound)?;
        let stored: ChannelSessionRecord = decode(row, "record_json")?;
        // A retried completed recovery must not suppress executions submitted
        // after the first successful review.
        if json(&stored)? == json(&current)? {
            return Ok(current);
        }
        if json(&stored)? != json(&expected)? {
            return Err(Error::Conflict("channel session changed during recovery"));
        }
        // Pending completions and active attempts are the only generations
        // that can still create output. Materialize the identities because
        // inserting their receipts consumes the corresponding pending rows.
        let route = ChannelOutputRoute::from_authority(&expected.authority, expected.session_id);
        tx.execute(statement(
            "WITH candidates AS MATERIALIZED (
                SELECT delivery_id FROM agena_channel_pending_outputs WHERE session_id=?1
                UNION ALL
                SELECT ?2||':'||a.execution_id||':'||a.generation
                FROM agena_attempts a
                CROSS JOIN agena_executions e ON e.id=a.execution_id AND e.current_generation=a.generation
                WHERE a.session_id=?1 AND a.phase IN ('queued','running')
             )
             INSERT INTO agena_integration_outbox
                (id,binding_id,state,attempts,next_attempt_ms,created_at_ms,error,route_json)
             SELECT delivery_id,?2,'skipped',0,0,?3,'execution predates compatible channel recovery',?4
             FROM candidates WHERE true ON CONFLICT(id) DO NOTHING",
            vec![
                current.session_id.into(),
                current.binding_id.to_string().into(),
                now.into(),
                json(&route)?.into(),
            ],
        ))
        .await
        .map_err(error)?;
        tx.execute(statement(
            "UPDATE agena_integration_outbox SET state='skipped',
                error='delivery predates compatible channel recovery'
             WHERE binding_id=? AND (state='queued' OR state='uncertain')
               AND json_extract(route_json,'$.session_id')=?
               AND json_extract(route_json,'$.epoch')<?",
            vec![
                current.binding_id.to_string().into(),
                current.session_id.into(),
                current.authority.epoch.into(),
            ],
        ))
        .await
        .map_err(error)?;
        let updated = tx.execute(statement("UPDATE agena_channel_sessions SET scope_key=?,epoch=?,record_json=? WHERE session_id=? AND binding_id=? AND scope_key=? AND epoch=?",
            vec![current.scope_key.clone().into(), current.authority.epoch.into(), json(&current)?.into(), current.session_id.into(), current.binding_id.to_string().into(), expected.scope_key.into(), expected.authority.epoch.into()])).await.map_err(error)?;
        if updated.rows_affected() != 1 {
            return Err(Error::Conflict("channel session changed during recovery"));
        }
        tx.commit().await.map_err(error)?;
        Ok(current)
    }

    pub(super) async fn channel_audit_read(&self, id: Uuid) -> Result<Vec<ChannelAccessAudit>> {
        self.reader.query_all(statement("SELECT audit_json FROM agena_channel_access_audit WHERE binding_id=? ORDER BY at_ms DESC,request_key LIMIT 100",vec![id.to_string().into()])).await.map_err(error)?.into_iter().map(|r|decode(r,"audit_json")).collect()
    }
    pub(super) async fn channel_sessions_read(
        &self,
        id: Uuid,
    ) -> Result<Vec<ChannelSessionRecord>> {
        self.reader.query_all(statement("SELECT record_json FROM agena_channel_sessions WHERE binding_id=? ORDER BY epoch DESC,session_id DESC LIMIT 2048",vec![id.to_string().into()])).await.map_err(error)?.into_iter().map(|r|decode(r,"record_json")).collect()
    }
    pub(super) async fn channel_session_read(
        &self,
        id: Uuid,
        key: &str,
    ) -> Result<Option<ChannelSessionRecord>> {
        self.reader
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
        tx.execute(statement("INSERT INTO agena_channel_sessions(session_id,binding_id,scope_key,epoch,record_json) VALUES(?,?,?,?,?)",vec![record.session_id.into(),record.binding_id.to_string().into(),record.scope_key.clone().into(),record.authority.epoch.into(),json(&record)?.into()])).await.map_err(error)?;
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
    if policy.per_user_daily_limit == 0 {
        return Ok(true);
    }
    let result = db
        .execute(statement(
            "INSERT INTO agena_channel_user_budget(binding_id,actor,utc_day,used) VALUES(?,?,?,1)
         ON CONFLICT(binding_id,actor,utc_day) DO UPDATE SET used=used+1 WHERE used<?",
            vec![
                b.id.to_string().into(),
                event.actor.clone().into(),
                day.into(),
                i64::from(policy.per_user_daily_limit).into(),
            ],
        ))
        .await
        .map_err(error)?;
    Ok(result.rows_affected() == 1)
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
                db.execute(statement("INSERT INTO agena_channel_access_audit(binding_id,request_key,input_json,result_json,audit_json,at_ms) VALUES(?,?,?,?,?,?)",vec![b.id.to_string().into(),audit.request_key.clone().into(),json(event)?.into(),pack_receipt(b)?.into(),json(&audit)?.into(),now.into()])).await.map_err(error)?;
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
        insert_event(db, &record).await?;
    }
    Ok(())
}
