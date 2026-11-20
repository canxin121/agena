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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedChannelRoom {
    pub resource: String,
    pub kind: ConversationKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ChannelParticipant {
    pub actor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub first_seen_at_ms: i64,
    pub last_seen_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ChannelParticipantPage {
    pub participants: Vec<ChannelParticipant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelParticipantsQuery {
    pub resource: String,
    pub kind: ConversationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
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
    fn verified_channel_message<'event>(
        &self,
        event: &'event ExternalEvent,
        now: i64,
    ) -> std::result::Result<(&ChannelAccessPolicy, &'event ChannelEnvelope), &'static str> {
        if !agena_domain::valid_channel_id(&event.actor) {
            return Err("invalid_or_reserved_sender_identity");
        }
        let Some(policy) = &self.spec.channel_policy else {
            return Err("channel_policy_required");
        };
        let Some(channel) = &event.channel else {
            return Err("verified_channel_context_required");
        };
        if channel.identity != policy.identity {
            return Err("tenant_or_account_mismatch");
        }
        let kind = channel.kind;
        if !policy.serves(kind) {
            return Err("conversation_kind_not_authorized");
        }
        if !self.spec.resource.matches(&event.resource) {
            return Err("conversation_not_authorized");
        }
        if policy.thread_mode == ChannelThreadMode::Exact && event.thread != self.spec.thread {
            return Err("thread_not_authorized");
        }
        if kind == ConversationKind::Direct
            && (event.thread.is_some() || policy.thread_mode != ChannelThreadMode::Exact)
        {
            return Err("direct_message_thread_invalid");
        }
        if kind == ConversationKind::Direct && policy.allow_unlisted_chat {
            return Err("direct_message_open_membership_invalid");
        }
        if !channel.actor_verified || !channel.is_member {
            return Err("platform_membership_not_verified");
        }
        if channel.verified_at_ms < now.saturating_sub(300000)
            || channel.verified_at_ms > now.saturating_add(30000)
        {
            return Err("membership_evidence_expired");
        }
        if event.bot {
            return Err("bot_echo");
        }
        Ok((policy, channel))
    }

    fn matches_channel_trigger(policy: &ChannelAccessPolicy, channel: &ChannelEnvelope) -> bool {
        channel.kind == ConversationKind::Direct
            || policy.trigger == ChannelTrigger::EveryMessage
            || channel.addressed_to_bot
    }

    /// An interaction is evidence for the directory independently of the
    /// local role and execution quota that decide whether the Bot will answer.
    pub fn observed_channel_kind(
        &self,
        event: &ExternalEvent,
        now: i64,
    ) -> Option<ConversationKind> {
        if self.spec.mode != IntegrationMode::Channel
            || event.event_type != "message"
            || !agena_domain::valid_channel_id(&event.resource)
            || self.spec.expired_at(now)
            || !self.event_in_activation(event, now)
        {
            return None;
        }
        let (policy, channel) = self.verified_channel_message(event, now).ok()?;
        Self::matches_channel_trigger(policy, channel).then_some(channel.kind)
    }

    pub fn channel_rejection(&self, event: &ExternalEvent, now: i64) -> Option<&'static str> {
        let (policy, channel) = match self.verified_channel_message(event, now) {
            Ok(message) => message,
            Err(reason) => return Some(reason),
        };
        if policy.grant.validate().is_err() {
            return Some("channel_grant_invalid");
        }
        if let Err(reason) = policy.member(&event.actor, now) {
            return Some(reason);
        }
        if !Self::matches_channel_trigger(policy, channel) {
            return Some("message_not_addressed_to_assistant");
        }
        None
    }
    pub fn authorizes_scope(&self, a: &ChannelAuthority, now: i64) -> bool {
        if a.grant.validate().is_err()
            || self.state != BindingState::Enabled
            || self.epoch != a.epoch
            || self.id.to_string() != a.binding_id
            || self.spec.expired_at(now)
            || !a.active_at(now)
        {
            return false;
        }
        let Some(p) = &self.spec.channel_policy else {
            return false;
        };
        if (!a.platform.is_empty() && a.platform != self.spec.provider)
            || a.identity != p.identity
            || !p.serves(a.kind)
            || !self.spec.resource.matches(&a.resource)
            || a.session_mode != p.session_mode_for(&a.actor, now)
            || a.source_admin != p.is_source_administrator(&a.actor, now)
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
            return p.allow_automated_reports
                && (a.grant == p.service_grant || !a.profile_fingerprint.is_empty());
        }
        if a.actor == "@shared" {
            // Shared rooms are the default for groups; the speaker is the
            // room, and capabilities are the binding's single grant.
            return a.role == ChannelRole::Member && a.grant == p.grant;
        }
        p.member(&a.actor, now).is_ok_and(|(role, _)| {
            role == a.role && (a.grant == p.grant || !a.profile_fingerprint.is_empty())
        })
    }
    pub fn authorizes_output(&self, route: &ChannelOutputRoute, now: i64) -> bool {
        if self.state != BindingState::Enabled
            || self.epoch != route.epoch
            || self.spec.expired_at(now)
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
            administrator_actors: Default::default(),
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
    fn source_administrator_authority_is_checked_against_live_local_policy() {
        let mut b = binding();
        let p = b.spec.channel_policy.as_mut().unwrap();
        p.administrator_actors.insert("alice".into());
        p.members.push(ChannelMember {
            actor: "bob".into(),
            role: ChannelRole::Member,
            expires_at_ms: None,
        });
        let scope = agena_domain::SessionChannelScope {
            binding_id: b.id.to_string(),
            identity: p.identity.account(),
            resource: "room".into(),
            thread: Some("thread-a".into()),
            kind: ConversationKind::Group,
            memory_workspace_root: "/private/admin".into(),
            private_paths: Default::default(),
        };
        let mut a = ChannelAuthority::for_speaker(
            &scope,
            b.epoch,
            "alice",
            ChannelRole::Owner,
            Default::default(),
            ChannelSessionMode::PerActor,
            1000000,
        );
        a.source_admin = true;
        assert!(b.authorizes_scope(&a, 2100));
        a.session_mode = ChannelSessionMode::PerConversation;
        assert!(!b.authorizes_scope(&a, 2100));
        a.session_mode = ChannelSessionMode::PerActor;
        a.actor = "bob".into();
        a.role = ChannelRole::Member;
        assert!(!b.authorizes_scope(&a, 2100));
        a.source_admin = false;
        a.session_mode = ChannelSessionMode::PerConversation;
        assert!(b.authorizes_scope(&a, 2100));
        a.actor = "alice".into();
        a.role = ChannelRole::Owner;
        a.source_admin = true;
        a.session_mode = ChannelSessionMode::PerActor;
        b.spec
            .channel_policy
            .as_mut()
            .unwrap()
            .administrator_actors
            .clear();
        assert!(!b.authorizes_scope(&a, 2100));
        b.spec
            .channel_policy
            .as_mut()
            .unwrap()
            .administrator_actors
            .insert("alice".into());
        b.epoch += 1;
        assert!(!b.authorizes_scope(&a, 2100));
    }

    #[test]
    fn verified_namespaces_mentions_and_membership_are_checked_independently() {
        let b = binding();
        let mut e = ExternalEvent {
            id: "one".into(),
            event_type: "message".into(),
            resource: "room".into(),
            actor: "alice".into(),
            actor_display_name: None,
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
