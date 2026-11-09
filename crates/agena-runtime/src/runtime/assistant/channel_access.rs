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
        if service
            .provider_identity(&binding.spec.provider)
            .ok()
            .as_deref()
            != Some(binding.provider_fingerprint.as_str())
            || service
                .source_permission_identity(binding.source_session_id)
                .await
                .ok()
                .as_deref()
                != Some(binding.permission_fingerprint.as_str())
        {
            return Err(
                "channel provider or source permissions changed; explicit review is required"
                    .into(),
            );
        }
        if !binding.authorizes_scope(authority, Utc::now().timestamp_millis()) {
            return Err("channel access revoked, expired or changed; no tool or execution is authorized by this old session".into());
        }
        Ok(())
    }
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
    pub(super) async fn channel_scope(
        &self,
        b: &IntegrationBinding,
        actor: &str,
        thread: Option<String>,
        now: i64,
    ) -> Result<ChannelSessionRecord> {
        let policy = b.spec.channel_policy.as_ref().ok_or(Error::Invalid(
            "channel permissions must be reviewed before use",
        ))?;
        let (principal, role, grant, expiry) = if actor == "@service" {
            if !policy.allow_automated_reports {
                return Err(Error::Invalid("automated channel reports are not enabled"));
            }
            (
                actor.to_owned(),
                ChannelRole::Operator,
                policy.service_grant.clone(),
                None,
            )
        } else {
            let (role, grant, expiry) = policy.member(actor, now).map_err(Error::Invalid)?;
            if policy.session_mode == ChannelSessionMode::SharedChat {
                (
                    "@shared".into(),
                    ChannelRole::Member,
                    ChannelGrant::default(),
                    None,
                )
            } else {
                (actor.to_owned(), role, grant, expiry)
            }
        };
        let privacy_key = hash(
            &json!({"binding":b.id,"provider":b.spec.provider,"identity":policy.identity,"resource":b.spec.resource,"thread":thread,"actor":principal}),
        )?;
        let scope_key =
            hash(&json!({"private_scope":privacy_key,"epoch":b.epoch,"role":role,"grant":grant}))?;
        if let Some(record) = self.integrations.channel_session(b.id, &scope_key).await? {
            if !b.authorizes_scope(&record.authority, now) {
                return Err(Error::Conflict(
                    "channel scope has expired or lost its role",
                ));
            }
            return Ok(record);
        }
        let private_root = agena_storage_layout::StorageLayout::current()
            .root()
            .join("channel-scopes")
            .join(b.id.to_string())
            .join(privacy_key);
        let memory = agena_storage::MemoryDir::from_workspace(&private_root);
        let mut private_paths = BTreeSet::from([private_root.clone()]);
        if grant.private_memory {
            private_paths.insert(memory.path().to_path_buf());
            private_paths.insert(memory.index_dir());
        }
        let authority = ChannelAuthority {
            binding_id: b.id.to_string(),
            epoch: b.epoch,
            actor: principal,
            role,
            identity: policy.identity.clone(),
            resource: b.spec.resource.clone(),
            thread,
            session_mode: policy.session_mode,
            grant,
            expires_at_ms: expiry
                .unwrap_or(b.spec.expires_at_ms)
                .min(b.spec.expires_at_ms),
            memory_workspace_root: private_root,
            private_paths,
            revoked: false,
        };
        let session_id = self
            .manager()?
            .ensure_channel_session(b.source_session_id, &scope_key, authority.clone())
            .await
            .map_err(runtime_error)?;
        self.integrations
            .save_channel_session(ChannelSessionRecord {
                binding_id: b.id,
                scope_key,
                session_id,
                authority,
                created_at_ms: now,
            })
            .await
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
            let scope = self
                .channel_scope(
                    &b,
                    "@service",
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
        let now = Utc::now().timestamp_millis();
        let policy = b
            .spec
            .channel_policy
            .as_ref()
            .ok_or(Error::Invalid("channel policy missing"))?;
        let (role, grant, _) = policy
            .member(&event.event.actor, now)
            .map_err(Error::Invalid)?;
        let words = text.split_whitespace().collect::<Vec<_>>();
        let mut current = b.clone();
        let response=match words.as_slice(){
            ["/access"]|["/access","whoami"]=>format!("Scope: {}/{}/{} · {:?} · thread {:?}\nUser: {} · local role: {}\nTools: {} · private memory: {}\nPlatform administrator labels do not grant instance privileges.",policy.identity.tenant,policy.identity.account,b.spec.resource,policy.identity.kind,event.event.thread,event.event.actor,role.as_str(),grant.tools.len(),grant.private_memory),
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
    use super::access_reply;
    #[test]
    fn long_member_list_fits_outbox_without_splitting_unicode() {
        let reply = access_reply("用户身份: administrator\n".repeat(1024));
        assert!(reply.len() < 24 * 1024);
        assert!(reply.ends_with("/integrations.]"));
        assert_eq!(access_reply("alice: member".into()), "alice: member");
    }
}
