//! Host-side channel RBAC, privacy partitions and revocation. No platform role
//! string or model-produced tool argument is an administrative credential.
use super::{AssistantRuntime, runtime_error};
use agena_assistant::*;
use chrono::Utc;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

fn hash(value: &impl serde::Serialize) -> Result<String> {
    let bytes = serde_json::to_vec(value).map_err(runtime_error)?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
struct ChannelVerifier {
    service: std::sync::Weak<AssistantRuntime>,
}
impl std::fmt::Debug for ChannelVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LiveChannelAccessVerifier")
    }
}
#[async_trait::async_trait]
impl agena_runtime_contracts::ChannelAccessVerifier for ChannelVerifier {
    async fn authorize(&self, authority: &ChannelAuthority) -> std::result::Result<(), String> {
        let id = authority
            .binding_id
            .parse()
            .map_err(|_| "invalid channel authority identity".to_owned())?;
        let service = self
            .service
            .upgrade()
            .ok_or_else(|| "channel authorization service stopped".to_owned())?;
        let binding = service
            .integrations
            .binding(id)
            .await
            .map_err(|_| "channel authorization state unavailable; access denied".to_owned())?;
        let source = service
            .verify_integration_boundary(&binding)
            .await
            .map_err(|error| error.to_string())?;
        if !binding.authorizes_scope(authority, Utc::now().timestamp_millis()) {
            return Err("channel access revoked, expired or changed; no tool or execution is authorized by this old session".into());
        }
        if authority.actor != "@shared" {
            let policy = binding
                .spec
                .channel_policy
                .as_ref()
                .ok_or_else(|| "channel policy unavailable".to_owned())?;
            let base_grant = if authority.actor == "@service" {
                &policy.service_grant
            } else {
                &policy.grant
            };
            let mut resolved = service
                .manager()
                .map_err(|error| error.to_string())?
                .access_control_config()
                .resolve(
                    &binding.spec.provider,
                    &authority.identity,
                    &authority.resource,
                    &authority.actor,
                    &agena_domain::PermissionConfig::default(),
                    base_grant,
                )?;
            apply_plugin_denials(&mut resolved.permission, &source.disabled_plugins);
            if resolved.fingerprint != authority.profile_fingerprint
                || resolved.grant != authority.grant
                || !authority.group_ids.is_empty()
                || resolved.profile_ids != authority.profile_ids
                || resolved.permission != authority.permission_overlay
                || resolved.complete != authority.permission_complete
            {
                return Err(
                    "access-control profile or user membership changed; submit a new message"
                        .into(),
                );
            }
        }
        Ok(())
    }
}
fn channel_scope_keys(
    binding: &IntegrationBinding,
    resource: &str,
    actor: &str,
    kind: agena_domain::ConversationKind,
    thread: Option<&str>,
    session_mode: ChannelSessionMode,
    source_admin: bool,
) -> Result<(String, String)> {
    let policy = binding
        .spec
        .channel_policy
        .as_ref()
        .ok_or(Error::Invalid("channel policy required"))?;
    let speaker = (session_mode == ChannelSessionMode::PerActor).then_some(actor);
    let mut privacy = json!({"binding":binding.id,"provider":binding.spec.provider,"identity":policy.identity,"kind":kind,"resource":resource,"thread":thread,"speaker":speaker});
    if source_admin {
        privacy["source_admin"] = json!(true);
    }
    let privacy_key = hash(&privacy)?;
    let scope_key = hash(&json!({"private_scope":privacy_key,"epoch":binding.epoch}))?;
    Ok((privacy_key, scope_key))
}

fn apply_plugin_denials(
    permission: &mut agena_domain::PermissionConfig,
    disabled: &BTreeSet<String>,
) {
    permission
        .plugins
        .extend(disabled.iter().cloned().map(|id| (id, false)));
}

fn compatible_channel_recovery(
    binding: &IntegrationBinding,
    previous: &ChannelSessionRecord,
    now: i64,
    access_control: &agena_domain::AccessControlConfig,
    disabled_plugins: &BTreeSet<String>,
) -> Result<ChannelSessionRecord> {
    let policy = binding
        .spec
        .channel_policy
        .as_ref()
        .ok_or(Error::Invalid("channel policy required"))?;
    let old = &previous.authority;
    if previous.binding_id != binding.id
        || old.binding_id != binding.id.to_string()
        || !old.active_at(now)
    {
        return Err(Error::Conflict(
            "the historical channel authority is expired or does not belong to this binding",
        ));
    }
    let (role, grant, expiry) = match old.actor.as_str() {
        "@service" if policy.allow_automated_reports => {
            (ChannelRole::Operator, &policy.service_grant, None)
        }
        "@shared" => (ChannelRole::Member, &policy.grant, None),
        actor => {
            let (role, expiry) = policy.member(actor, now).map_err(Error::Invalid)?;
            (role, &policy.grant, expiry)
        }
    };
    let resolved = if old.actor == "@shared" {
        None
    } else {
        let mut resolved = access_control
            .resolve(
                &binding.spec.provider,
                &policy.identity,
                &old.resource,
                &old.actor,
                &agena_domain::PermissionConfig::default(),
                grant,
            )
            .map_err(runtime_error)?;
        apply_plugin_denials(&mut resolved.permission, disabled_plugins);
        Some(resolved)
    };
    let expected_grant = resolved.as_ref().map(|r| &r.grant).unwrap_or(grant);
    if old.grant != *expected_grant
        || resolved.as_ref().is_some_and(|r| {
            r.fingerprint != old.profile_fingerprint
                || !old.group_ids.is_empty()
                || r.permission != old.permission_overlay
                || r.complete != old.permission_complete
        })
        || old.identity != policy.identity
        || old.session_mode != policy.session_mode_for(&old.actor, now)
        || old.source_admin != policy.is_source_administrator(&old.actor, now)
        || old.expires_at_ms != binding.spec.authority_expires_at(expiry)
    {
        return Err(Error::Conflict(
            "channel scope, tools, administrator capability or expiry changed; the old conversation cannot inherit the new authority",
        ));
    }
    let mut current = previous.clone();
    current.authority.epoch = binding.epoch;
    current.authority.role = role;
    current.authority.revoked = false;
    if !binding.authorizes_scope(&current.authority, now) {
        return Err(Error::Conflict(
            "the current channel policy no longer admits this historical conversation",
        ));
    }
    current.scope_key = channel_scope_keys(
        binding,
        &old.resource,
        &old.actor,
        old.kind,
        old.thread.as_deref(),
        old.session_mode,
        old.source_admin,
    )?
    .1;
    Ok(current)
}

fn access_reply(text: String) -> String {
    const PREVIEW_BYTES: usize = 16 * 1024;
    if text.len() <= PREVIEW_BYTES {
        return text;
    }
    let mut end = PREVIEW_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[Member list preview truncated. Inspect the complete policy in Agena /integrations.]",
        &text[..end]
    )
}

impl AssistantRuntime {
    pub(super) fn install_channel_verifier(self: &Arc<Self>) {
        if let Ok(manager) = self.manager() {
            manager.install_channel_verifier(Arc::new(ChannelVerifier {
                service: Arc::downgrade(self),
            }));
        }
    }
    pub(super) async fn channel_scope_for(
        &self,
        b: &IntegrationBinding,
        resource: &str,
        actor: &str,
        kind: agena_domain::ConversationKind,
        thread: Option<String>,
        now: i64,
    ) -> Result<ChannelSessionRecord> {
        if !b.spec.resource.matches(resource) {
            return Err(Error::Invalid("conversation is outside this binding"));
        }
        let policy = b.spec.channel_policy.as_ref().ok_or(Error::Invalid(
            "channel permissions must be reviewed before use",
        ))?;
        let (role, base_grant, expiry) = if actor == "@service" {
            if !policy.allow_automated_reports {
                return Err(Error::Invalid("automated channel reports are not enabled"));
            }
            (ChannelRole::Operator, policy.service_grant.clone(), None)
        } else {
            let (role, expiry) = policy.member(actor, now).map_err(Error::Invalid)?;
            (role, policy.grant.clone(), expiry)
        };
        let source_admin = policy.is_source_administrator(actor, now);
        let session_mode = policy.session_mode_for(actor, now);
        let (privacy_key, scope_key) = channel_scope_keys(
            b,
            resource,
            actor,
            kind,
            thread.as_deref(),
            session_mode,
            source_admin,
        )?;
        let manager = self.manager()?;
        let mut resolved = manager
            .access_control_config()
            .resolve(
                &b.spec.provider,
                &policy.identity,
                resource,
                actor,
                &agena_domain::PermissionConfig::default(),
                &base_grant,
            )
            .map_err(runtime_error)?;
        let source = self
            .source_permission_policy(b.source_session_id, b.spec.mode)
            .await?;
        apply_plugin_denials(&mut resolved.permission, &source.disabled_plugins);
        let private_root = agena_storage_layout::StorageLayout::current()
            .root()
            .join("channel-scopes")
            .join(b.id.to_string())
            .join(&privacy_key);
        // Preserve existing numeric/user-id memory directories. Only an
        // unsafe path component needs a stable opaque directory name.
        let speaker_key = if actor.is_empty()
            || matches!(actor, "." | "..")
            || actor.contains('/')
            || actor.contains('\\')
        {
            hash(&actor)?
        } else {
            actor.to_owned()
        };
        let speaker_root = agena_storage_layout::StorageLayout::current()
            .root()
            .join("channel-speakers")
            .join(b.id.to_string())
            .join(&privacy_key)
            .join(speaker_key);
        let memory = agena_storage::MemoryDir::from_workspace(&speaker_root);
        let mut private_paths = BTreeSet::from([private_root.clone(), speaker_root.clone()]);
        if source_admin || resolved.grant.private_memory {
            private_paths.insert(memory.path().to_path_buf());
            private_paths.insert(memory.index_dir());
        }
        let authority = ChannelAuthority {
            binding_id: b.id.to_string(),
            platform: b.spec.provider.clone(),
            group_ids: Vec::new(),
            profile_ids: resolved.profile_ids,
            profile_fingerprint: resolved.fingerprint,
            permission_overlay: resolved.permission,
            permission_complete: resolved.complete,
            epoch: b.epoch,
            actor: actor.to_owned(),
            role,
            identity: policy.identity.clone(),
            resource: resource.to_owned(),
            kind,
            thread,
            session_mode,
            grant: resolved.grant,
            source_admin,
            expires_at_ms: b.spec.authority_expires_at(expiry),
            memory_workspace_root: speaker_root.clone(),
            source_workspace_root: manager
                .session_workspace_root_optional(b.source_session_id)
                .await
                .map_err(runtime_error)?,
            private_paths,
            revoked: false,
        };
        let directory = speaker_root;
        tokio::task::spawn_blocking(move || agena_storage_layout::secure_directory(&directory))
            .await
            .map_err(runtime_error)?
            .map_err(runtime_error)?;
        if let Some(mut record) = self.integrations.channel_session(b.id, &scope_key).await? {
            if manager
                .session_exists(record.session_id)
                .await
                .map_err(runtime_error)?
            {
                // A cached transcript is shared; its first sender is never
                // evidence for the next execution's identity or permissions.
                record.authority = authority;
                return Ok(record);
            }
            self.integrations
                .forget_channel_session(b.id, &scope_key)
                .await?;
        }
        let mut baseline = authority.clone();
        if session_mode == ChannelSessionMode::PerConversation && actor != "@service" {
            baseline.actor = "@shared".into();
            baseline.role = ChannelRole::Member;
            baseline.group_ids.clear();
            baseline.profile_ids.clear();
            baseline.profile_fingerprint.clear();
            baseline.permission_overlay = Default::default();
            baseline.permission_complete = false;
            baseline.grant = policy.grant.clone();
            baseline.expires_at_ms = b.spec.authority_expires_at(None);
            baseline.memory_workspace_root = private_root.clone();
            baseline.private_paths = BTreeSet::from([private_root]);
        }
        let session_id = manager
            .ensure_channel_session(b.source_session_id, &scope_key, baseline.clone())
            .await
            .map_err(runtime_error)?;
        let mut record = self
            .integrations
            .save_channel_session(ChannelSessionRecord {
                binding_id: b.id,
                scope_key,
                session_id,
                authority: baseline,
                created_at_ms: now,
            })
            .await?;
        record.authority = authority;
        Ok(record)
    }
    pub(super) async fn ensure_report_session(
        &self,
        mut b: IntegrationBinding,
    ) -> Result<IntegrationBinding> {
        if b.spec.mode == IntegrationMode::Channel
            && b.spec
                .channel_policy
                .as_ref()
                .is_some_and(|p| p.allow_automated_reports)
            && b.channel_session_id.is_none()
        {
            let Some(resource) = b.spec.resource.single().map(str::to_owned) else {
                // A binding serving many conversations has no single room to
                // hold automated reports; reports are scoped per event instead.
                return Ok(b);
            };
            let scope = self
                .channel_scope_for(
                    &b,
                    &resource,
                    "@service",
                    agena_domain::ConversationKind::Group,
                    b.spec.thread.clone(),
                    Utc::now().timestamp_millis(),
                )
                .await?;
            b.channel_session_id = Some(scope.session_id);
            let revision = b.revision;
            return self.integrations.save_binding(b, revision).await;
        }
        Ok(b)
    }
    /// Resume only a locally reviewed, identical transport/permission boundary.
    /// The admission gate is held by control_integration throughout this method.
    pub(super) async fn review_channel_connection(
        &self,
        mut binding: IntegrationBinding,
        provider: IntegrationProvider,
        now: i64,
    ) -> Result<IntegrationBinding> {
        if binding.spec.mode != IntegrationMode::Channel {
            return Err(Error::Invalid(
                "history review applies to channel connections only",
            ));
        }
        if provider.fingerprint != binding.provider_fingerprint {
            return Err(Error::Conflict(
                "the provider transport or connection identity changed; the existing conversation cannot be resumed under a different boundary",
            ));
        }
        let source_policy = self
            .source_permission_policy(binding.source_session_id, binding.spec.mode)
            .await?;
        if source_policy.fingerprint != binding.permission_fingerprint {
            return Err(Error::Conflict(
                "the source permission boundary changed; review cannot reuse this conversation under different capabilities",
            ));
        }
        if provider.adapter.kind != AdapterKind::Channel
            || provider.adapter.channel_context_version != Some(1)
            || binding.spec.poll_seconds < provider.adapter.minimum_poll_seconds
        {
            return Err(Error::Invalid(
                "the current provider no longer satisfies the verified channel contract",
            ));
        }
        if binding.spec.expired_at(now) || binding.spec.total_budget_exhausted(binding.admitted) {
            return Err(Error::Conflict(
                "the channel grant expired or exhausted its budget",
            ));
        }
        let policy = binding
            .spec
            .channel_policy
            .as_ref()
            .ok_or(Error::Invalid("channel policy required"))?;
        policy.validate(now).map_err(Error::Policy)?;
        let source = self
            .manager()?
            .get_session(binding.source_session_id)
            .await
            .map_err(runtime_error)?;
        if source.execution.channel_authority.is_some() {
            return Err(Error::Invalid(
                "channel history review requires a local source conversation",
            ));
        }
        let expected_revision = binding.revision;
        binding.state = BindingState::Enabled;
        binding.epoch += 1;
        binding.revision += 1;
        binding.updated_at_ms = now;
        binding.next_poll_ms = now;
        binding.last_error = None;
        let records = self.integrations.channel_sessions(binding.id).await?;
        let manager = self.manager()?;
        let mut recoveries = Vec::new();
        let mut scope_keys = BTreeSet::new();
        for record in records {
            if !manager
                .session_exists(record.session_id)
                .await
                .map_err(runtime_error)?
            {
                continue;
            }
            let current = compatible_channel_recovery(
                &binding,
                &record,
                now,
                &manager.access_control_config(),
                &source_policy.disabled_plugins,
            )?;
            if !scope_keys.insert(current.scope_key.clone()) {
                return Err(Error::Conflict(
                    "multiple historical conversations claim the same channel scope; select the conversation to restore before reviewing",
                ));
            }
            recoveries.push((record, current));
        }
        // Publish the reviewed boundary first so the kernel's live verifier can
        // check each new authority. Old records still have the old epoch and
        // cannot dispatch or export while recovery is in progress.
        let binding = self
            .integrations
            .review_channel_binding(binding, expected_revision, "instance_operator", now)
            .await?;
        for (previous, current) in recoveries {
            let recovery = async {
                let scope_key = current.scope_key.clone();
                manager
                    .recover_channel_session_authority(
                        previous.session_id,
                        previous.authority.clone(),
                        current.authority.clone(),
                        &scope_key,
                        || async {
                            if let Err(error) = self
                                .integrations
                                .recover_channel_session(previous, current, now)
                                .await
                            {
                                let mut failed = binding.clone();
                                failed.state = BindingState::Faulted;
                                failed.revision += 1;
                                failed.last_error = Some(format!(
                                    "compatible channel recovery did not complete: {error}"
                                ));
                                self.integrations
                                    .save_binding(failed, binding.revision)
                                    .await
                                    .map_err(|error| {
                                        crate::AppError::PersistenceConflict(error.to_string())
                                    })?;
                                return Err(crate::AppError::PersistenceConflict(
                                    error.to_string(),
                                ));
                            }
                            Ok(())
                        },
                    )
                    .await
                    .map_err(runtime_error)?;
                Ok::<(), Error>(())
            }
            .await;
            if let Err(error) = recovery {
                let mut failed = binding.clone();
                failed.state = BindingState::Faulted;
                failed.revision += 1;
                failed.last_error = Some(format!(
                    "compatible channel recovery did not complete: {error}"
                ));
                let _ = self
                    .integrations
                    .save_binding(failed, binding.revision)
                    .await;
                return Err(error);
            }
        }
        self.reconcile_integration_executions(&binding).await?;
        self.wake.notify_one();
        Ok(binding)
    }

    pub(super) async fn revoke_old_channel_runs(&self, b: &IntegrationBinding) -> Result<()> {
        let manager = self.manager()?;
        let now = Utc::now().timestamp_millis();
        for record in self.integrations.channel_sessions(b.id).await? {
            if record.authority.epoch <= b.epoch && !b.authorizes_scope(&record.authority, now) {
                // A cleanup from an older policy/receipt cannot retire a
                // session admitted by a later authorization update.
                // The persisted grant already rejects all new tool preflights.
                // These calls additionally stop currently running providers and
                // their native Subagent trees.
                if let Err(error) = manager.retire_channel_session(record.session_id).await {
                    tracing::warn!(session=record.session_id,%error,"revoked channel cleanup deferred");
                }
            }
        }
        Ok(())
    }
    pub(super) async fn update_channel_access(
        &self,
        source: i64,
        id: Uuid,
        r: ChannelPolicyUpdate,
    ) -> Result<IntegrationBinding> {
        self.scoped_binding(source, id).await?;
        let b = {
            let _lock = self.integration_gate.lock().await;
            self.integrations
                .update_channel_policy(id, r, "instance_operator", Utc::now().timestamp_millis())
                .await?
        };
        let current = self.integrations.binding(id).await?;
        self.revoke_old_channel_runs(&current).await?;
        self.wake.notify_one();
        Ok(b)
    }
    pub(super) async fn channel_admin_message(
        &self,
        b: &IntegrationBinding,
        event: &IntegrationEvent,
    ) -> Result<bool> {
        let text = event.event.text.trim();
        if text != "/access" && !text.starts_with("/access ") {
            return Ok(false);
        }
        if b.spec.is_telegram_channel() {
            return Ok(true);
        }
        let now = Utc::now().timestamp_millis();
        let policy = b
            .spec
            .channel_policy
            .as_ref()
            .ok_or(Error::Invalid("channel policy missing"))?;
        let (role, _) = policy
            .member(&event.event.actor, now)
            .map_err(Error::Invalid)?;
        let grant = &policy.grant;
        let source_admin = policy.is_source_administrator(&event.event.actor, now);
        let tools = if source_admin {
            "source Bot catalog".to_owned()
        } else {
            grant.allowed_tools().len().to_string()
        };
        let words = text.split_whitespace().collect::<Vec<_>>();
        let mut current = b.clone();
        let response=match words.as_slice(){
            ["/access"]|["/access","whoami"]=>format!("Scope: {}/{}/{} · {:?} · thread {:?}\nUser: {} · local role: {}\nTools: {} · private memory: {}\nPlatform administrator labels do not grant instance privileges.",policy.identity.tenant,policy.identity.account,b.spec.resource.display(),event.event.channel.as_ref().map(|c| c.kind.as_str()).unwrap_or("unknown"),event.event.thread,event.event.actor,role.as_str(),tools,source_admin || grant.private_memory),
            ["/access","members"] if role.can_manage()=>policy.members.iter().map(|m|format!("{}: {}{}",m.actor,m.role.as_str(),if m.expires_at_ms.is_some_and(|expiry|now>=expiry){" (expired)"}else{""})).collect::<Vec<_>>().join("\n"),
            ["/access","set",target,requested]=>{
                let assigned=ChannelRole::parse(requested);
                let result=assigned.ok_or("unknown_role").and_then(|next|policy.remote_assignment(&event.event.actor,target,next,now));
                match result{
                    Ok(policy)=>{
                        current=self.integrations.update_channel_policy(b.id,ChannelPolicyUpdate{expected_revision:b.revision,request_key:format!("channel:{}",hash(&event.event.id)?),policy},&format!("channel:{}",event.event.actor),now).await?;
                        format!("Updated {} to {} in this conversation only. Access revision {}. Previous sessions and queued replies are revoked.",target,requested,current.revision)
                    },Err(reason)=>format!("Access denied: {reason}. No membership changed."),
                }
            }
            ["/access","revoke",target]=>{
                match policy.remote_assignment(&event.event.actor,target,ChannelRole::Blocked,now){
                    Ok(policy)=>{
                        current=self.integrations.update_channel_policy(b.id,ChannelPolicyUpdate{expected_revision:b.revision,request_key:format!("channel:{}",hash(&event.event.id)?),policy},&format!("channel:{}",event.event.actor),now).await?;
                        format!("Revoked {target} in this conversation. Access revision {}.",current.revision)
                    },Err(reason)=>format!("Access denied: {reason}. No membership changed."),
                }
            }
            _=>"Commands: /access whoami; administrators: /access members, /access set USER member|operator|administrator|blocked, /access revoke USER. You cannot modify your own role or a peer/higher role.".into(),
        };
        self.integrations
            .enqueue_delivery(IntegrationDelivery {
                id: format!("{}:access:{}", b.id, hash(&event.event.id)?),
                binding_id: b.id,
                text: access_reply(response),
                state: "queued".into(),
                attempts: 0,
                next_attempt_ms: 0,
                remote_id: None,
                created_at_ms: now,
                error: None,
                route: Some(ChannelOutputRoute {
                    actor: event.event.actor.clone(),
                    identity: policy.identity.clone(),
                    resource: event.event.resource.clone(),
                    thread: event.event.thread.clone(),
                    kind: event
                        .event
                        .channel
                        .as_ref()
                        .map(|channel| channel.kind)
                        .unwrap_or(agena_domain::ConversationKind::Group),
                    epoch: current.epoch,
                    session_id: None,
                }),
            })
            .await?;
        // No LLM is invoked for authorization commands. Cancellation is
        // driven after the persisted decision; even a crash cannot restore it.
        if current.epoch != b.epoch {
            self.revoke_old_channel_runs(&current).await?;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn recovery_fixture() -> (IntegrationBinding, ChannelSessionRecord) {
        let policy = ChannelAccessPolicy {
            identity: ChannelIdentity {
                tenant: "telegram".into(),
                account: "bot".into(),
            },
            kinds: [agena_domain::ConversationKind::Direct].into(),
            session_mode: ChannelSessionMode::PerActor,
            thread_mode: ChannelThreadMode::Exact,
            trigger: ChannelTrigger::DirectOrMention,
            members: vec![ChannelMember {
                actor: "42".into(),
                role: ChannelRole::Owner,
                expires_at_ms: None,
            }],
            administrator_actors: Default::default(),
            allow_unlisted_chat: false,
            grant: Default::default(),
            per_user_daily_limit: 0,
            allow_automated_reports: false,
            service_grant: Default::default(),
        };
        let spec = IntegrationSpec {
            title: "Existing direct chat".into(),
            provider: "agena.telegram".into(),
            mode: IntegrationMode::Channel,
            config: json!({"bot_id":"telegram-bot"}),
            resource: "42".into(),
            thread: None,
            event_types: vec!["message".into()],
            actors: vec![],
            instruction: String::new(),
            poll_seconds: 10,
            expires_at_ms: 1_000_000,
            daily_limit: 100,
            total_limit: 1000,
            max_execution_seconds: 60,
            use_source_permissions: true,
            channel_policy: Some(policy.clone()),
        };
        let mut binding = IntegrationBinding::new(
            Some(1),
            None,
            9,
            "original".into(),
            spec,
            "legacy".into(),
            1000,
        )
        .unwrap();
        binding.state = BindingState::Enabled;
        binding.epoch = 4;
        binding.revision = 5;
        let authority = ChannelAuthority {
            platform: String::new(),
            group_ids: Vec::new(),
            profile_ids: Vec::new(),
            profile_fingerprint: String::new(),
            permission_overlay: Default::default(),
            permission_complete: false,
            source_workspace_root: None,
            binding_id: binding.id.to_string(),
            epoch: 1,
            actor: "42".into(),
            role: ChannelRole::Member,
            identity: policy.identity,
            resource: "42".into(),
            kind: agena_domain::ConversationKind::Direct,
            thread: None,
            session_mode: ChannelSessionMode::PerActor,
            grant: Default::default(),
            source_admin: false,
            expires_at_ms: 1_000_000,
            memory_workspace_root: "/private/original".into(),
            private_paths: ["/private/original".into()].into(),
            revoked: false,
        };
        let record = ChannelSessionRecord {
            binding_id: binding.id,
            scope_key: "old-scope".into(),
            session_id: 11,
            authority,
            created_at_ms: 1000,
        };
        (binding, record)
    }
    #[test]
    fn compatible_recovery_preserves_original_scope_and_session_when_only_role_and_epoch_change() {
        let (binding, previous) = recovery_fixture();
        let current = compatible_channel_recovery(
            &binding,
            &previous,
            2000,
            &Default::default(),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(current.session_id, 11);
        assert_eq!(current.created_at_ms, previous.created_at_ms);
        assert_eq!(current.authority.epoch, 4);
        assert_eq!(current.authority.role, ChannelRole::Owner);
        assert_eq!(current.authority.grant, previous.authority.grant);
        assert_eq!(
            current.authority.memory_workspace_root,
            previous.authority.memory_workspace_root
        );
        assert_eq!(
            current.authority.private_paths,
            previous.authority.private_paths
        );
        assert_ne!(current.scope_key, previous.scope_key);
        assert!(binding.authorizes_scope(&current.authority, 2000));
        assert_eq!(
            current.scope_key,
            channel_scope_keys(
                &binding,
                "42",
                "42",
                agena_domain::ConversationKind::Direct,
                None,
                ChannelSessionMode::PerActor,
                false
            )
            .unwrap()
            .1
        );
    }
    #[test]
    fn compatible_recovery_rejects_changed_capabilities_expiry_and_conversation_scope() {
        let changes: &[fn(&mut IntegrationBinding)] = &[
            |b| {
                b.spec
                    .channel_policy
                    .as_mut()
                    .unwrap()
                    .grant
                    .tools
                    .insert("agena.fs.read".into());
            },
            |b| {
                b.spec
                    .channel_policy
                    .as_mut()
                    .unwrap()
                    .administrator_actors
                    .insert("42".into());
            },
            |b| b.spec.channel_policy.as_mut().unwrap().members[0].role = ChannelRole::Blocked,
            |b| b.spec.channel_policy.as_mut().unwrap().members[0].expires_at_ms = Some(10_000),
            |b| b.spec.expires_at_ms = 2_000_000,
            |b| b.spec.resource = "another-chat".into(),
            |b| b.spec.channel_policy.as_mut().unwrap().identity.account = "another-bot".into(),
            |b| {
                b.spec.channel_policy.as_mut().unwrap().session_mode =
                    ChannelSessionMode::PerConversation
            },
            |b| b.spec.thread = Some("another-topic".into()),
        ];
        for change in changes {
            let (mut binding, previous) = recovery_fixture();
            change(&mut binding);
            assert!(
                compatible_channel_recovery(
                    &binding,
                    &previous,
                    2000,
                    &Default::default(),
                    &Default::default()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn long_member_list_fits_outbox_without_splitting_unicode() {
        let reply = access_reply("用户身份: administrator\n".repeat(1024));
        assert!(reply.len() < 24 * 1024);
        assert!(reply.ends_with("/integrations.]"));
        assert_eq!(access_reply("alice: member".into()), "alice: member");
    }
}
