//! Channel identity checks, routing facts and audited policy-change contracts.
use crate::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelPolicyUpdate {
    pub expected_revision: i64,
    pub request_key: String,
    pub policy: ChannelAccessPolicy,
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ChannelAccessAudit {
    #[schemars(with = "String")]
    pub binding_id: Uuid,
    pub request_key: String,
    pub actor: String,
    pub revision: i64,
    pub epoch: i64,
    pub at_ms: i64,
    pub before: Option<ChannelAccessPolicy>,
    pub after: ChannelAccessPolicy,
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ChannelSessionRecord {
    #[schemars(with = "String")]
    pub binding_id: Uuid,
    pub scope_key: String,
    pub session_id: i64,
    pub authority: ChannelAuthority,
    pub created_at_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelOutputRoute {
    pub actor: String,
    pub identity: ChannelIdentity,
    pub resource: String,
    pub thread: Option<String>,
    /// Room shape of the destination, so a multi-surface account can address
    /// the right one and the host can recheck the binding still serves it.
    #[serde(default = "default_kind")]
    pub kind: ConversationKind,
    pub epoch: i64,
    pub session_id: Option<i64>,
}

fn default_kind() -> ConversationKind {
    ConversationKind::Group
}
impl ChannelOutputRoute {
    pub fn from_authority(a: &ChannelAuthority, session: i64) -> Self {
        Self {
            actor: a.actor.clone(),
            identity: a.identity.clone(),
            resource: a.resource.clone(),
            thread: a.thread.clone(),
            kind: a.kind,
            epoch: a.epoch,
            session_id: Some(session),
        }
    }
}
impl IntegrationBinding {
    pub fn channel_rejection(&self, event: &ExternalEvent, now: i64) -> Option<&'static str> {
        if !agena_domain::valid_channel_id(&event.actor) {
            return Some("invalid_or_reserved_sender_identity");
        }
        let Some(policy) = &self.spec.channel_policy else {
            return Some("channel_policy_required");
        };
        let Some(channel) = &event.channel else {
            return Some("verified_channel_context_required");
        };
        if channel.identity != policy.identity {
            return Some("tenant_or_account_mismatch");
        }
        let kind = channel.kind;
        if !policy.serves(kind) {
            return Some("conversation_kind_not_authorized");
        }
        if !self.spec.resource.matches(&event.resource) {
            return Some("conversation_not_authorized");
        }
        if policy.thread_mode == ChannelThreadMode::Exact && event.thread != self.spec.thread {
            return Some("thread_not_authorized");
        }
        if kind == ConversationKind::Direct
            && (event.thread.is_some() || policy.thread_mode != ChannelThreadMode::Exact)
        {
            return Some("direct_message_thread_invalid");
        }
        if kind == ConversationKind::Direct && policy.allow_unlisted_chat {
            return Some("direct_message_open_membership_invalid");
        }
        if !channel.actor_verified || !channel.is_member {
            return Some("platform_membership_not_verified");
        }
        if channel.verified_at_ms < now - 300000 || channel.verified_at_ms > now + 30000 {
            return Some("membership_evidence_expired");
        }
        if event.bot {
            return Some("bot_echo");
        }
        if policy.grant.validate().is_err() {
            return Some("channel_grant_invalid");
        }
        if let Err(reason) = policy.member(&event.actor, now) {
            return Some(reason);
        }
        if kind != ConversationKind::Direct
            && policy.trigger == ChannelTrigger::DirectOrMention
            && !channel.addressed_to_bot
        {
            return Some("message_not_addressed_to_assistant");
        }
        None
    }
    pub fn authorizes_scope(&self, a: &ChannelAuthority, now: i64) -> bool {
        if a.grant.validate().is_err()
            || self.state != BindingState::Enabled
            || self.epoch != a.epoch
            || self.id.to_string() != a.binding_id
            || now >= self.spec.expires_at_ms
            || !a.active_at(now)
        {
            return false;
        }
        let Some(p) = &self.spec.channel_policy else {
            return false;
        };
        if a.identity != p.identity
            || !p.serves(a.kind)
            || !self.spec.resource.matches(&a.resource)
            || a.session_mode != p.session_mode
        {
            return false;
        }
        if a.kind == ConversationKind::Direct
            && (a.thread.is_some() || p.thread_mode != ChannelThreadMode::Exact)
        {
            return false;
        }
        if p.thread_mode == ChannelThreadMode::Exact && a.thread != self.spec.thread {
            return false;
        }
        if a.actor == "@service" {
            return p.allow_automated_reports && a.grant == p.service_grant;
        }
        if a.actor == "@shared" {
            // Shared rooms are the default for groups; the speaker is the
            // room, and capabilities are the binding's single grant.
            return a.role == ChannelRole::Member && a.grant == p.grant;
        }
        p.member(&a.actor, now)
            .is_ok_and(|(role, _)| role == a.role && a.grant == p.grant)
    }
    pub fn authorizes_output(&self, route: &ChannelOutputRoute, now: i64) -> bool {
        if self.state != BindingState::Enabled
            || self.epoch != route.epoch
            || now >= self.spec.expires_at_ms
        {
            return false;
        }
        let Some(p) = &self.spec.channel_policy else {
            return false;
        };
        if route.identity != p.identity || !self.spec.resource.matches(&route.resource) {
            return false;
        }
        if p.thread_mode == ChannelThreadMode::Exact && route.thread != self.spec.thread {
            return false;
        }
        match route.actor.as_str() {
            "@service" => p.allow_automated_reports,
            "@shared" => true,
            actor => p.member(actor, now).is_ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> IntegrationBinding {
        let policy = ChannelAccessPolicy {
            identity: ChannelIdentity {
                tenant: "one".into(),
                account: "bot".into(),
            },
            kinds: Default::default(),
            session_mode: ChannelSessionMode::PerConversation,
            thread_mode: ChannelThreadMode::PerThread,
            trigger: ChannelTrigger::DirectOrMention,
            members: vec![ChannelMember {
                actor: "alice".into(),
                role: ChannelRole::Owner,
                expires_at_ms: None,
            }],
            allow_unlisted_chat: false,
            grant: Default::default(),
            per_user_daily_limit: 10,
            allow_automated_reports: false,
            service_grant: Default::default(),
        };
        let spec = IntegrationSpec {
            title: "group".into(),
            provider: "test.channel".into(),
            mode: IntegrationMode::Channel,
            config: serde_json::json!({}),
            resource: "room".into(),
            thread: None,
            event_types: vec!["message".into()],
            actors: vec![],
            instruction: String::new(),
            poll_seconds: 1,
            expires_at_ms: 1000000,
            daily_limit: 100,
            total_limit: 1000,
            max_execution_seconds: 300,
            use_source_permissions: true,
            channel_policy: Some(policy),
        };
        let mut b =
            IntegrationBinding::new(Some(1), None, 1, "create".into(), spec, "fp".into(), 1000)
                .unwrap();
        b.state = BindingState::Enabled;
        b.epoch = 1;
        b.activated_at_ms = Some(1000);
        b
    }
    #[test]
    fn verified_namespaces_mentions_and_membership_are_checked_independently() {
        let b = binding();
        let mut e = ExternalEvent {
            id: "one".into(),
            event_type: "message".into(),
            resource: "room".into(),
            actor: "alice".into(),
            thread: Some("thread-a".into()),
            bot: false,
            occurred_at_ms: 2000,
            text: "hello".into(),
            data: serde_json::Value::Null,
            channel: Some(ChannelEnvelope {
                identity: b.spec.channel_policy.as_ref().unwrap().identity.clone(),
                kind: ConversationKind::Group,
                actor_verified: true,
                is_member: true,
                addressed_to_bot: true,
                verified_at_ms: 2000,
            }),
        };
        assert!(b.rejection(&e, 2100).is_none());
        e.channel.as_mut().unwrap().identity.tenant = "two".into();
        assert_eq!(b.rejection(&e, 2100), Some("tenant_or_account_mismatch"));
        e.channel.as_mut().unwrap().identity.tenant = "one".into();
        e.channel.as_mut().unwrap().addressed_to_bot = false;
        assert_eq!(
            b.rejection(&e, 2100),
            Some("message_not_addressed_to_assistant")
        );
        e.channel.as_mut().unwrap().addressed_to_bot = true;
        e.actor = "unknown-admin".into();
        e.data = serde_json::json!({"role":"owner"});
        assert_eq!(b.rejection(&e, 2100), Some("member_not_authorized"));
    }
}
