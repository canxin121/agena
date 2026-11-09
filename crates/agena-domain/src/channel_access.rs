//! Host-owned authorization for externally authenticated chat identities.
//! Provider metadata identifies the sender/scope; it never assigns Agena roles.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    #[default]
    Direct,
    Group,
    PublicChannel,
    PrivateChannel,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelIdentity {
    pub tenant: String,
    pub account: String,
    pub kind: ChannelKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelEnvelope {
    pub identity: ChannelIdentity,
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChannelSessionMode {
    #[default]
    PerUser,
    SharedChat,
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
            if matches!(
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
            ) {
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
    /// option never confers Member's optional privileged grant.
    #[serde(default)]
    pub allow_unlisted_chat: bool,
    #[serde(default)]
    #[schemars(with = "BTreeMap<String,ChannelGrant>")]
    pub grants: BTreeMap<ChannelRole, ChannelGrant>,
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
        if self.identity.kind == ChannelKind::Direct
            && (self.thread_mode != ChannelThreadMode::Exact
                || self.session_mode != ChannelSessionMode::PerUser
                || self.allow_unlisted_chat)
        {
            return Err(
                "direct messages require exact-thread, per-user sessions and explicit participants"
                    .into(),
            );
        }
        if self
            .grants
            .get(&ChannelRole::Blocked)
            .is_some_and(ChannelGrant::has_effects)
        {
            return Err("blocked users cannot receive capabilities".into());
        }
        for grant in self.grants.values() {
            grant.validate()?;
        }
        self.service_grant.validate()?;
        if self.service_grant.private_memory {
            return Err("automated report sessions cannot use human private memories".into());
        }
        if !self.allow_automated_reports && self.service_grant.has_effects() {
            return Err("service capabilities require explicit automated-report consent".into());
        }
        if self.session_mode == ChannelSessionMode::SharedChat
            && self.grants.values().any(ChannelGrant::has_effects)
        {
            return Err("shared chat has no tools, work or private memory; use per-user sessions for privileged capabilities".into());
        }
        Ok(())
    }
    pub fn member(
        &self,
        actor: &str,
        now: i64,
    ) -> Result<(ChannelRole, ChannelGrant, Option<i64>), &'static str> {
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
            return Ok((
                m.role,
                self.grants.get(&m.role).cloned().unwrap_or_default(),
                m.expires_at_ms,
            ));
        }
        if self.allow_unlisted_chat {
            Ok((ChannelRole::Member, ChannelGrant::default(), None))
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
        let (role, _, _) = self.member(actor, now)?;
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

/// Immutable capability fence stored with a session and inherited by workers,
/// agents and forks. Current membership is rechecked by the host verifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelAuthority {
    pub binding_id: String,
    pub epoch: i64,
    pub actor: String,
    pub role: ChannelRole,
    pub identity: ChannelIdentity,
    pub resource: String,
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
    pub fn active_at(&self, now: i64) -> bool {
        !self.revoked && now < self.expires_at_ms
    }
    pub fn allows_tool(&self, canonical: &str, now: i64) -> bool {
        self.active_at(now) && self.grant.allowed_tools().contains(canonical)
    }
    pub fn permission_fence(&self) -> crate::PermissionConfig {
        use crate::{
            NetworkPermissionConfig, PathAccessModes, PathPermissionConfig, PermissionMode,
        };
        let deny = PermissionMode::Deny;
        let allow = PermissionMode::Allow;
        let mut rules = indexmap::IndexMap::new();
        for path in &self.private_paths {
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
                rules: self
                    .grant
                    .network_hosts
                    .iter()
                    .map(|host| (host.clone(), allow))
                    .collect(),
            }),
            ..Default::default()
        }
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
                kind: ChannelKind::Group,
            },
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
            grants: Default::default(),
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
    fn deny_beats_public_membership_and_default_role_never_inherits_tools() {
        let mut p = policy();
        p.allow_unlisted_chat = true;
        p.grants.insert(
            ChannelRole::Member,
            ChannelGrant {
                private_memory: true,
                ..Default::default()
            },
        );
        assert!(!p.member("unlisted", 1).unwrap().1.private_memory);
        p.members.push(ChannelMember {
            actor: "blocked".into(),
            role: ChannelRole::Blocked,
            expires_at_ms: None,
        });
        assert!(p.member("blocked", 1).is_err());
    }
    #[test]
    fn shared_context_cannot_mix_privileged_sessions() {
        let mut p = policy();
        p.session_mode = ChannelSessionMode::SharedChat;
        p.grants.insert(
            ChannelRole::Owner,
            ChannelGrant {
                tools: BTreeSet::from(["agena.agent.spawn".into()]),
                ..Default::default()
            },
        );
        assert!(p.validate(1).is_err());
        p.grants.clear();
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
    fn role_labels_do_not_grant_machine_permissions() {
        let p = policy();
        for actor in ["owner", "admin", "member"] {
            assert!(p.member(actor, 1).unwrap().1.allowed_tools().is_empty());
        }
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
