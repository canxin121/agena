//! Host-owned authorization for externally authenticated chat identities.
//! Provider metadata identifies the sender/scope; it never assigns Agena roles.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

/// A chat platform account owned by one binding.
///
/// The conversation kind is not part of the account: one account serves direct
/// chats, groups and broadcast channels at once, and each message carries its
/// own kind. Keeping the kind out of the account identity is what lets one
/// binding serve every room shape the adapter reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelAccount {
    pub tenant: String,
    pub account: String,
}

/// The chat transport identity: tenant plus account. No room shape here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelIdentity {
    pub tenant: String,
    pub account: String,
}

impl ChannelIdentity {
    pub fn account(&self) -> ChannelAccount {
        ChannelAccount {
            tenant: self.tenant.clone(),
            account: self.account.clone(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelEnvelope {
    pub identity: ChannelIdentity,
    /// The shape of the room this specific message arrived in. It lives on the
    /// message, not on the account: one account serves direct chats, groups and
    /// broadcast channels at once.
    pub kind: crate::ConversationKind,
    /// The installed adapter verifies these facts with its platform. A user
    /// cannot obtain membership just by putting an ID/role in message text.
    pub actor_verified: bool,
    pub is_member: bool,
    pub addressed_to_bot: bool,
    pub verified_at_ms: i64,
}
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ChannelRole {
    Blocked,
    #[default]
    Member,
    Operator,
    Administrator,
    Owner,
}
impl ChannelRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Member => "member",
            Self::Operator => "operator",
            Self::Administrator => "administrator",
            Self::Owner => "owner",
        }
    }
    pub fn can_manage(self) -> bool {
        matches!(self, Self::Administrator | Self::Owner)
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "blocked" => Some(Self::Blocked),
            "member" => Some(Self::Member),
            "operator" => Some(Self::Operator),
            "administrator" => Some(Self::Administrator),
            "owner" => Some(Self::Owner),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelMember {
    pub actor: String,
    pub role: ChannelRole,
    #[serde(default)]
    pub expires_at_ms: Option<i64>,
}
/// How one conversation's sessions are partitioned.
///
/// `PerConversation` is the default: a group is one session shared by all of
/// its members, a direct chat is one session per participant, and a channel is
/// one session per channel. Context is never merged across conversations or
/// across platform accounts.
///
/// `PerActor` additionally splits a single conversation by speaker. It is an
/// explicit opt-in for rooms whose members must not share a transcript.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChannelSessionMode {
    #[default]
    PerConversation,
    PerActor,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChannelThreadMode {
    #[default]
    Exact,
    PerThread,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChannelTrigger {
    #[default]
    DirectOrMention,
    EveryMessage,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ChannelGrant {
    /// Exact canonical tool IDs only; empty means no tools. Scope-preserving
    /// memory/companion helpers are derived from the booleans below, not freely
    /// assigned management-tool names.
    pub tools: BTreeSet<String>,
    /// Fine-grained MCP server -> exact remote tool names. The generic MCP
    /// gateway and connection-management tools are never directly grantable.
    pub mcp_tools: BTreeMap<String, BTreeSet<String>>,
    pub private_memory: bool,
    pub source_workspace: bool,
    /// No network by default, including loopback/private targets. Explicit
    /// destination rules still intersect the source session's permission cap.
    pub network_hosts: BTreeSet<String>,
}
impl ChannelGrant {
    pub fn allows_mcp_tool(&self, server: &str, tool: &str) -> bool {
        self.mcp_tools
            .get(server)
            .is_some_and(|tools| tools.contains(tool))
    }

    pub fn has_effects(&self) -> bool {
        !self.tools.is_empty()
            || !self.mcp_tools.is_empty()
            || self.private_memory
            || self.source_workspace
            || !self.network_hosts.is_empty()
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.tools.len() > 64 || self.network_hosts.len() > 32 || self.mcp_tools.len() > 16 {
            return Err("channel grant exceeds tool/network bounds".into());
        }
        for tool in &self.tools {
            if tool.len() > 160
                || tool.split('.').count() < 3
                || !tool
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            {
                return Err("channel tools require exact canonical IDs; wildcards and aliases are not accepted".into());
            }
            let plugin = tool.split('.').take(2).collect::<Vec<_>>().join(".");
            if [
                "agena.mcp.connections.",
                "agena.provider.",
                "agena.marketplace.",
                "agena.runtime.",
                "agena.auth.",
                "agena.oauth.",
            ]
            .iter()
            .any(|prefix| tool.starts_with(prefix))
            {
                return Err(format!(
                    "{tool} changes instance-level configuration and cannot be delegated through a channel role"
                ));
            }
            // Cross-project session management can be explicitly delegated
            // to a Bot-owned channel. Other Bot administration stays local.
            if tool != "agena.bot.sessions"
                && matches!(
                    plugin.as_str(),
                    "agena.config"
                        | "agena.settings"
                        | "agena.permission"
                        | "agena.permissions"
                        | "agena.plugin"
                        | "agena.plugins"
                        | "agena.secret"
                        | "agena.secrets"
                        | "agena.session"
                        | "agena.bot"
                        | "agena.companion"
                        | "agena.memory"
                        | "agena.cron"
                        | "agena.monitor"
                        | "agena.mcp"
                )
            {
                return Err(format!(
                    "{tool} cannot be granted directly to a channel; use the scoped capability controls"
                ));
            }
        }
        for (server, tools) in &self.mcp_tools {
            if !valid_channel_id(server)
                || server.contains('*')
                || tools.is_empty()
                || tools.len() > 32
                || tools
                    .iter()
                    .any(|tool| !valid_channel_id(tool) || tool.contains('*'))
            {
                return Err("MCP grants require exact server/tool identities, at most 16 servers and 32 tools per server".into());
            }
        }
        for host in &self.network_hosts {
            if !valid_channel_id(host) || host.contains(['*', '/', '@', '?', '#']) {
                return Err("network grants require exact host or host:port identities".into());
            }
        }
        Ok(())
    }
    pub fn allowed_tools(&self) -> BTreeSet<String> {
        let mut tools = self.tools.clone();
        if !self.mcp_tools.is_empty() {
            tools.insert("agena.mcp.tools.call".into());
            tools.insert("agena.mcp.tools.search".into());
        }
        if self.private_memory {
            for name in ["list", "get", "search", "write", "delete"] {
                tools.insert(format!("agena.memory.{name}"));
            }
            tools.insert("agena.companion.manage".into());
        }
        tools
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelAccessPolicy {
    pub identity: ChannelIdentity,
    /// Room shapes this binding may serve. Empty means every shape the adapter
    /// reports. The shape of any individual message is still decided per
    /// message, and `Direct` keeps its stricter session rules wherever it
    /// appears.
    #[serde(default)]
    pub kinds: BTreeSet<crate::ConversationKind>,
    #[serde(default)]
    pub session_mode: ChannelSessionMode,
    #[serde(default)]
    pub thread_mode: ChannelThreadMode,
    #[serde(default)]
    pub trigger: ChannelTrigger,
    /// Explicit local assignments override the open-member option, including
    /// a Blocked entry. Remote platform administrator claims are ignored.
    pub members: Vec<ChannelMember>,
    /// Unlisted platform members can only get chat-only Member access. This
    /// option never confers the binding capability grant.
    #[serde(default)]
    pub allow_unlisted_chat: bool,
    /// Capabilities available to every admitted member of this binding.
    ///
    /// This is deliberately one grant per binding rather than one per role.
    /// A conversation's transcript is shared by everyone speaking in it, so
    /// letting two roles hold different tools would let a low-privilege
    /// message drive a tool whose output the whole room then reads. One grant
    /// makes that leak structurally impossible; roles continue to govern
    /// membership management only.
    #[serde(default)]
    pub grant: ChannelGrant,
    #[serde(default = "daily")]
    pub per_user_daily_limit: u32,
    /// Explicit automated posts use a separate service session, never borrow
    /// a human owner's private chat or memories.
    #[serde(default)]
    pub allow_automated_reports: bool,
    #[serde(default)]
    pub service_grant: ChannelGrant,
}
fn daily() -> u32 {
    50
}
pub fn valid_channel_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.chars().any(|c| c.is_control() || c.is_whitespace())
        && !matches!(value, "*" | "@service" | "@shared" | "@unattributed")
}
impl ChannelAccessPolicy {
    /// True when this binding may serve the given room shape.
    pub fn serves(&self, kind: crate::ConversationKind) -> bool {
        self.kinds.is_empty() || self.kinds.contains(&kind)
    }

    /// True when this binding is limited to one-to-one conversations.
    pub fn direct_only(&self) -> bool {
        !self.kinds.is_empty()
            && self
                .kinds
                .iter()
                .all(|kind| *kind == crate::ConversationKind::Direct)
    }

    pub fn validate(&self, now: i64) -> Result<(), String> {
        if !valid_channel_id(&self.identity.tenant) || !valid_channel_id(&self.identity.account) {
            return Err("channel tenant and account must be exact stable IDs".into());
        }
        if self.members.is_empty()
            || self.members.len() > 128
            || !(1..=1000).contains(&self.per_user_daily_limit)
        {
            return Err(
                "channel policy needs 1–128 assignments and a daily user limit of 1–1000".into(),
            );
        }
        let mut seen = BTreeSet::new();
        let mut owner = false;
        for m in &self.members {
            if !valid_channel_id(&m.actor) || !seen.insert(&m.actor) {
                return Err("channel member IDs must be valid and unique".into());
            }
            if m.role == ChannelRole::Owner && m.expires_at_ms.is_none_or(|expiry| expiry > now) {
                owner = true;
            }
        }
        if !owner {
            return Err("channel policy needs at least one unexpired local owner".into());
        }
        // A direct-only binding keeps the strict one-partner rules. A binding
        // that also serves groups may use per-thread and open-member options,
        // which stay gated per message by `serves`/`admits_direct`.
        if self.kinds.contains(&crate::ConversationKind::Direct)
            && (self.thread_mode != ChannelThreadMode::Exact || self.allow_unlisted_chat)
        {
            return Err(
                "direct messages require exact-thread sessions and explicit participants".into(),
            );
        }
        self.grant.validate()?;
        self.service_grant.validate()?;
        if self.service_grant.private_memory {
            return Err("automated report sessions cannot use human private memories".into());
        }
        if !self.allow_automated_reports && self.service_grant.has_effects() {
            return Err("service capabilities require explicit automated-report consent".into());
        }
        Ok(())
    }
    pub fn member(
        &self,
        actor: &str,
        now: i64,
    ) -> Result<(ChannelRole, Option<i64>), &'static str> {
        if !valid_channel_id(actor) {
            return Err("invalid_or_reserved_sender_identity");
        }
        if let Some(m) = self.members.iter().find(|m| m.actor == actor) {
            if m.role == ChannelRole::Blocked {
                return Err("member_blocked");
            }
            if m.expires_at_ms.is_some_and(|expiry| now >= expiry) {
                return Err("membership_expired");
            }
            return Ok((m.role, m.expires_at_ms));
        }
        if self.allow_unlisted_chat {
            Ok((ChannelRole::Member, None))
        } else {
            Err("member_not_authorized")
        }
    }
    pub fn remote_assignment(
        &self,
        actor: &str,
        target: &str,
        new_role: ChannelRole,
        now: i64,
    ) -> Result<Self, &'static str> {
        let (role, _) = self.member(actor, now)?;
        if !role.can_manage() {
            return Err("members_manage_denied");
        }
        if !valid_channel_id(target) || actor == target || new_role >= role {
            return Err("role_escalation_denied");
        }
        if self
            .members
            .iter()
            .find(|m| m.actor == target)
            .is_some_and(|m| m.role >= role)
        {
            return Err("peer_or_higher_role_denied");
        }
        let mut next = self.clone();
        if let Some(member) = next.members.iter_mut().find(|m| m.actor == target) {
            member.role = new_role;
        } else {
            if next.members.len() >= 128 {
                return Err("member_capacity_reached");
            }
            next.members.push(ChannelMember {
                actor: target.into(),
                role: new_role,
                expires_at_ms: None,
            });
        }
        Ok(next)
    }
}

/// The isolation facts of one channel conversation.
///
/// This is stored with the session: which binding, which external conversation
/// and thread, and which private directories belong to it. It deliberately
/// carries no speaker identity and no capabilities — a group conversation has
/// many speakers, and each message brings its own authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionChannelScope {
    pub binding_id: String,
    pub identity: ChannelAccount,
    pub resource: String,
    pub thread: Option<String>,
    pub kind: crate::ConversationKind,
    pub memory_workspace_root: PathBuf,
    pub private_paths: BTreeSet<PathBuf>,
}

impl SessionChannelScope {
    pub fn conversation_kind(&self) -> crate::ConversationKind {
        self.kind
    }

    /// Resolve the directories this conversation may touch through the
    /// permission fence. Capabilities are added separately, per execution.
    pub fn permission_fence(&self, granted_paths: &BTreeSet<PathBuf>) -> crate::PermissionConfig {
        permission_fence(&self.private_paths, granted_paths, &BTreeSet::new())
    }
}

/// The authority of one message's speaker, valid for that execution only.
///
/// Workers, agents and forks inherit the authority of the execution that
/// spawned them, never the authority of another speaker in the same room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelAuthority {
    pub binding_id: String,
    pub epoch: i64,
    pub actor: String,
    pub role: ChannelRole,
    pub identity: ChannelIdentity,
    pub resource: String,
    pub kind: crate::ConversationKind,
    pub thread: Option<String>,
    pub session_mode: ChannelSessionMode,
    pub grant: ChannelGrant,
    pub expires_at_ms: i64,
    pub memory_workspace_root: PathBuf,
    pub private_paths: BTreeSet<PathBuf>,
    #[serde(default)]
    pub revoked: bool,
}
impl ChannelAuthority {
    /// Build the per-execution authority for one speaker in a scope.
    pub fn for_speaker(
        scope: &SessionChannelScope,
        epoch: i64,
        actor: impl Into<String>,
        role: ChannelRole,
        grant: ChannelGrant,
        session_mode: ChannelSessionMode,
        expires_at_ms: i64,
    ) -> Self {
        Self {
            binding_id: scope.binding_id.clone(),
            epoch,
            actor: actor.into(),
            role,
            identity: ChannelIdentity {
                tenant: scope.identity.tenant.clone(),
                account: scope.identity.account.clone(),
            },
            resource: scope.resource.clone(),
            kind: scope.kind,
            thread: scope.thread.clone(),
            session_mode,
            grant,
            expires_at_ms,
            memory_workspace_root: scope.memory_workspace_root.clone(),
            private_paths: scope.private_paths.clone(),
            revoked: false,
        }
    }

    /// The conversation scope this authority belongs to.
    pub fn scope(&self) -> SessionChannelScope {
        SessionChannelScope {
            binding_id: self.binding_id.clone(),
            identity: self.identity.account(),
            resource: self.resource.clone(),
            thread: self.thread.clone(),
            kind: self.kind,
            memory_workspace_root: self.memory_workspace_root.clone(),
            private_paths: self.private_paths.clone(),
        }
    }

    pub fn active_at(&self, now: i64) -> bool {
        !self.revoked && now < self.expires_at_ms
    }
    pub fn allows_tool(&self, canonical: &str, now: i64) -> bool {
        self.active_at(now) && self.grant.allowed_tools().contains(canonical)
    }
    pub fn permission_fence(&self) -> crate::PermissionConfig {
        permission_fence(
            &self.private_paths,
            &BTreeSet::new(),
            &self.grant.network_hosts,
        )
    }
}

/// Shared fence construction: private paths stay reachable, everything outside
/// is denied, and only explicitly granted network destinations open.
fn permission_fence(
    private_paths: &BTreeSet<PathBuf>,
    granted_paths: &BTreeSet<PathBuf>,
    network_hosts: &BTreeSet<String>,
) -> crate::PermissionConfig {
    use crate::{NetworkPermissionConfig, PathAccessModes, PathPermissionConfig, PermissionMode};
    let deny = PermissionMode::Deny;
    let allow = PermissionMode::Allow;
    let mut rules = indexmap::IndexMap::new();
    for path in private_paths.iter().chain(granted_paths.iter()) {
        let path = path.to_string_lossy().into_owned();
        let modes = PathAccessModes {
            read: Some(allow),
            write: Some(allow),
        };
        rules.insert(path.clone(), modes.clone());
        rules.insert(format!("{path}/**"), modes);
    }
    crate::PermissionConfig {
        path: Some(PathPermissionConfig {
            workspace: Some(PathAccessModes {
                read: Some(allow),
                write: Some(allow),
            }),
            external: Some(PathAccessModes {
                read: Some(deny),
                write: Some(deny),
            }),
            rules,
        }),
        network: Some(NetworkPermissionConfig {
            internet: Some(deny),
            private: Some(deny),
            loopback: Some(deny),
            rules: network_hosts
                .iter()
                .map(|host| (host.clone(), allow))
                .collect(),
        }),
        ..Default::default()
    }
}

#[allow(dead_code)]
fn _assert_permission_fence_shape(_: &crate::PermissionConfig) {}

impl From<&ChannelAuthority> for SessionChannelScope {
    fn from(value: &ChannelAuthority) -> Self {
        value.scope()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> ChannelAccessPolicy {
        ChannelAccessPolicy {
            identity: ChannelIdentity {
                tenant: "tenant-a".into(),
                account: "bot-a".into(),
            },
            kinds: BTreeSet::from([crate::ConversationKind::Group]),
            session_mode: Default::default(),
            thread_mode: Default::default(),
            trigger: Default::default(),
            members: vec![
                ChannelMember {
                    actor: "owner".into(),
                    role: ChannelRole::Owner,
                    expires_at_ms: None,
                },
                ChannelMember {
                    actor: "admin".into(),
                    role: ChannelRole::Administrator,
                    expires_at_ms: None,
                },
                ChannelMember {
                    actor: "member".into(),
                    role: ChannelRole::Member,
                    expires_at_ms: None,
                },
            ],
            allow_unlisted_chat: false,
            grant: Default::default(),
            per_user_daily_limit: 50,
            allow_automated_reports: false,
            service_grant: Default::default(),
        }
    }
    #[test]
    fn admins_cannot_promote_self_or_peer_or_grant_owner() {
        let p = policy();
        assert!(p.validate(1).is_ok());
        assert!(
            p.remote_assignment("admin", "admin", ChannelRole::Owner, 1)
                .is_err()
        );
        assert!(
            p.remote_assignment("admin", "owner", ChannelRole::Blocked, 1)
                .is_err()
        );
        assert!(
            p.remote_assignment("admin", "member", ChannelRole::Administrator, 1)
                .is_err()
        );
        assert!(
            p.remote_assignment("member", "stranger", ChannelRole::Member, 1)
                .is_err()
        );
        assert!(
            p.remote_assignment("admin", "member", ChannelRole::Operator, 1)
                .is_ok()
        );
    }
    #[test]
    fn deny_beats_public_membership_and_unlisted_chat_never_adds_capability() {
        let mut p = policy();
        p.allow_unlisted_chat = true;
        p.grant = ChannelGrant {
            private_memory: true,
            ..Default::default()
        };
        // Unlisted speakers are admitted, but the binding grant is what the
        // session actually receives; admission alone never widens a grant.
        assert!(p.member("unlisted", 1).is_ok());
        p.members.push(ChannelMember {
            actor: "blocked".into(),
            role: ChannelRole::Blocked,
            expires_at_ms: None,
        });
        assert!(p.member("blocked", 1).is_err());
    }
    #[test]
    fn one_grant_per_binding_keeps_shared_rooms_safe() {
        // Roles no longer carry capabilities, so two speakers in one group
        // cannot hold different tools. `validate` accepts a privileged grant
        // because every admitted member shares it.
        let mut p = policy();
        p.grant = ChannelGrant {
            tools: BTreeSet::from(["agena.agent.spawn".into()]),
            ..Default::default()
        };
        assert!(p.validate(1).is_ok());
        p.session_mode = ChannelSessionMode::PerActor;
        assert!(p.validate(1).is_ok());
    }
    #[test]
    fn native_subagents_need_exact_tools_and_no_implicit_background_grant() {
        let grant = ChannelGrant {
            tools: BTreeSet::from(["agena.agent.spawn".into(), "agena.agent.cancel".into()]),
            ..Default::default()
        };
        assert!(grant.validate().is_ok());
        assert_eq!(grant.allowed_tools(), grant.tools);
        assert!(ChannelGrant::default().allowed_tools().is_empty());
        assert!(
            serde_json::from_value::<ChannelGrant>(serde_json::json!({"background_work":true}))
                .is_err()
        );
    }
    #[test]
    fn bot_session_management_can_be_explicitly_granted_to_shared_channels() {
        let mut p = policy();
        assert!(!p.grant.allowed_tools().contains("agena.bot.sessions"));
        p.grant.tools.insert("agena.bot.sessions".into());
        assert!(p.validate(1).is_ok());
        assert!(p.grant.allowed_tools().contains("agena.bot.sessions"));
        p.session_mode = ChannelSessionMode::PerActor;
        assert!(p.validate(1).is_ok());
        p.grant.tools.insert("agena.bot.connections".into());
        assert!(p.validate(1).is_err());
    }

    #[test]
    fn role_labels_do_not_grant_machine_permissions() {
        let p = policy();
        for actor in ["owner", "admin", "member"] {
            assert!(p.member(actor, 1).is_ok());
        }
        assert!(!p.grant.has_effects());
        let grant = ChannelGrant {
            tools: BTreeSet::from(["agena.bot.connections".into()]),
            ..Default::default()
        };
        assert!(grant.validate().is_err());
    }
}

#[cfg(test)]
mod mcp_grant_tests {
    use super::*;
    #[test]
    fn gateway_does_not_grant_unlisted_mcp_tools_or_connection_management() {
        for tool in [
            "agena.mcp.tools.call",
            "agena.mcp.connections.create",
            "agena.mcp.connections.update",
            "agena.mcp.connections.list",
            "agena.mcp.servers.reconnect",
        ] {
            assert!(
                ChannelGrant {
                    tools: BTreeSet::from([tool.into()]),
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        let grant = ChannelGrant {
            mcp_tools: BTreeMap::from([("docs".into(), BTreeSet::from(["search".into()]))]),
            ..Default::default()
        };
        assert!(grant.validate().is_ok());
        assert!(grant.has_effects());
        assert!(grant.allowed_tools().contains("agena.mcp.tools.call"));
        assert!(grant.allows_mcp_tool("docs", "search"));
        assert!(!grant.allows_mcp_tool("docs", "delete"));
        assert!(!grant.allows_mcp_tool("other", "search"));
    }
}
