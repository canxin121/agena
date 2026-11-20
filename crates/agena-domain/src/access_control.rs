//! Named, instance-owned permission configurations and exact channel assignments.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{ChannelGrant, ChannelIdentity, PermissionConfig};

/// Reserved outside the existing user-defined ID grammar.
pub const DEFAULT_PERMISSION_PROFILE_ID: &str = "@default";

static DEFAULT_PERMISSION_PROFILE: LazyLock<PermissionProfile> =
    LazyLock::new(|| PermissionProfile {
        name: "Default permissions".into(),
        permission: PermissionConfig::global_default(),
        complete: true,
        ..PermissionProfile::default()
    });

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccessControlConfig {
    pub profiles: BTreeMap<String, PermissionProfile>,
    pub groups: BTreeMap<String, PermissionUserGroup>,
    pub users: Vec<PermissionUserBinding>,
    pub conversations: Vec<PermissionConversationBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionProfile {
    pub name: String,
    pub description: String,
    pub revision: u64,
    pub permission: PermissionConfig,
    /// Complete configurations replace earlier named policies. The legacy
    /// format is retained for existing installations that authored overlays.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub complete: bool,
    /// Optional replacement for the channel capability selection. Permission
    /// overlays still run through the source's independent permission ceiling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant: Option<ChannelGrant>,
}

impl Default for PermissionProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            revision: 1,
            permission: PermissionConfig::default(),
            complete: false,
            grant: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionUserGroup {
    pub name: String,
    /// Lower priority is applied first. Conflicting assignments at the same
    /// priority are rejected rather than depending on storage iteration order.
    pub priority: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionUserBinding {
    pub platform: String,
    pub tenant: String,
    pub account: String,
    pub actor: String,
    /// Optional exact conversation for this sender's assignment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    pub group_ids: BTreeSet<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "PermissionConfig::is_empty")]
    pub overrides: PermissionConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionConversationBinding {
    pub platform: String,
    pub tenant: String,
    pub account: String,
    pub resource: String,
    pub default_profile_id: String,
}

/// The host resolves this value from verified ingress facts. It is never a
/// model-supplied argument, and it does not replace the execution's live ceiling.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedAccessControl {
    pub permission: PermissionConfig,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub complete: bool,
    pub grant: ChannelGrant,
    pub group_ids: Vec<String>,
    pub profile_ids: Vec<String>,
    /// Content-derived revision of only the assignments used by this subject.
    /// Unrelated users' changes do not revoke this execution.
    pub fingerprint: String,
}

impl AccessControlConfig {
    pub fn with_default_profile(mut self) -> Self {
        self.profiles
            .entry(DEFAULT_PERMISSION_PROFILE_ID.into())
            .or_insert_with(|| DEFAULT_PERMISSION_PROFILE.clone());
        self
    }

    pub fn standalone_permission(&self, id: &str) -> Result<PermissionConfig, String> {
        Ok(self.profile(id)?.permission.standalone())
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
            && self.groups.is_empty()
            && self.users.is_empty()
            && self.conversations.is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.profiles.len()
            - usize::from(self.profiles.contains_key(DEFAULT_PERMISSION_PROFILE_ID))
            > 256
            || self.groups.len() > 256
            || self.users.len() > 10000
            || self.conversations.len() > 4096
        {
            return Err("access_control exceeds profile/group/member/conversation limits".into());
        }
        for (id, profile) in &self.profiles {
            if id != DEFAULT_PERMISSION_PROFILE_ID {
                validate_id("profile", id)?;
            }
            validate_name("profile", &profile.name)?;
            if profile.revision == 0 || profile.description.len() > 4096 {
                return Err(format!(
                    "profile {id:?} needs a positive revision and a bounded description"
                ));
            }
            if let Some(grant) = &profile.grant {
                grant
                    .validate()
                    .map_err(|error| format!("profile {id:?}: {error}"))?;
            }
        }
        for (id, group) in &self.groups {
            validate_id("group", id)?;
            validate_name("group", &group.name)?;
            if let Some(profile) = &group.profile_id {
                self.profile(profile)?;
            }
        }
        let mut identities = BTreeSet::new();
        for user in &self.users {
            validate_identity(&user.platform, &user.tenant, &user.account, &user.actor)?;
            if let Some(resource) = &user.resource {
                validate_identity(&user.platform, &user.tenant, &user.account, resource)?;
            }
            if !identities.insert((
                &user.platform,
                &user.tenant,
                &user.account,
                &user.actor,
                &user.resource,
            )) {
                return Err(format!(
                    "duplicate access_control user {:?}/{:?}/{:?}/{:?} in {:?}",
                    user.platform, user.tenant, user.account, user.actor, user.resource
                ));
            }
            if let Some(profile) = &user.profile_id {
                self.profile(profile)?;
            }
            let groups = self.user_groups(user)?;
            for (index, (left_id, left)) in groups.iter().enumerate() {
                for (right_id, right) in groups.iter().skip(index + 1) {
                    if left.priority != right.priority {
                        continue;
                    }
                    if let (Some(left_profile), Some(right_profile)) =
                        (&left.profile_id, &right.profile_id)
                    {
                        let a = self.profile(left_profile)?;
                        let b = self.profile(right_profile)?;
                        if ((a.complete || b.complete)
                            && (a.complete != b.complete
                                || a.permission != b.permission
                                || a.grant != b.grant))
                            || permissions_conflict(&a.permission, &b.permission)
                            || matches!((&a.grant, &b.grant), (Some(x), Some(y)) if x != y)
                        {
                            return Err(format!(
                                "user {:?} belongs to conflicting groups {left_id:?} and {right_id:?} at priority {}; choose distinct priorities",
                                user.actor, left.priority
                            ));
                        }
                    }
                }
            }
        }
        let mut conversations = BTreeSet::new();
        for conversation in &self.conversations {
            validate_identity(
                &conversation.platform,
                &conversation.tenant,
                &conversation.account,
                &conversation.resource,
            )?;
            if !conversations.insert((
                &conversation.platform,
                &conversation.tenant,
                &conversation.account,
                &conversation.resource,
            )) {
                return Err(format!(
                    "duplicate access_control conversation {:?}",
                    conversation.resource
                ));
            }
            self.profile(&conversation.default_profile_id)?;
        }
        Ok(())
    }

    /// The final channel default and personal overrides use the existing
    /// PermissionConfig merge semantics, including atomic network replacement.
    /// Callers apply the source and runtime ceilings after this resolution.
    pub fn resolve(
        &self,
        platform: &str,
        identity: &ChannelIdentity,
        resource: &str,
        actor: &str,
        base_permission: &PermissionConfig,
        base_grant: &ChannelGrant,
    ) -> Result<ResolvedAccessControl, String> {
        self.validate()?;
        let conversation = self.conversations.iter().find(|item| {
            item.platform == platform
                && item.tenant == identity.tenant
                && item.account == identity.account
                && item.resource == resource
        });
        let mut matching_users = self.users.iter().filter(|item| {
            item.platform == platform
                && item.tenant == identity.tenant
                && item.account == identity.account
                && item.actor == actor
        });
        let user = matching_users.clone().find(|item| item.resource.is_none());
        let conversation_user =
            matching_users.find(|item| item.resource.as_deref() == Some(resource));
        let mut resolved = ResolvedAccessControl {
            permission: base_permission.clone(),
            grant: base_grant.clone(),
            ..Default::default()
        };
        let mut selected_profiles = Vec::new();
        let mut selected_groups = Vec::new();
        if let Some(conversation) = conversation {
            self.apply_profile(
                &conversation.default_profile_id,
                &mut resolved,
                &mut selected_profiles,
            )?;
        }
        for user in [user, conversation_user].into_iter().flatten() {
            for (id, group) in self.user_groups(user)? {
                if !resolved.group_ids.contains(id) {
                    resolved.group_ids.push(id.clone());
                }
                selected_groups.push((id, group));
                if let Some(profile) = &group.profile_id {
                    self.apply_profile(profile, &mut resolved, &mut selected_profiles)?;
                }
            }
            if let Some(profile) = &user.profile_id {
                self.apply_profile(profile, &mut resolved, &mut selected_profiles)?;
            }
            resolved.permission.merge_from(user.overrides.clone());
        }
        if conversation.is_some() || user.is_some() || conversation_user.is_some() {
            let selections = (conversation, user, selected_groups, selected_profiles);
            let payload = if let Some(scoped) = conversation_user {
                serde_json::to_vec(&(selections, scoped))
            } else {
                serde_json::to_vec(&selections)
            }
            .map_err(|error| format!("cannot fingerprint permission assignment: {error}"))?;
            resolved.fingerprint = Sha256::digest(payload)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
        }
        Ok(resolved)
    }

    fn profile(&self, id: &str) -> Result<&PermissionProfile, String> {
        self.profiles
            .get(id)
            .or_else(|| (id == DEFAULT_PERMISSION_PROFILE_ID).then(|| &*DEFAULT_PERMISSION_PROFILE))
            .ok_or_else(|| {
                format!("permission profile {id:?} is missing or still referenced after deletion")
            })
    }

    fn user_groups<'a>(
        &'a self,
        user: &'a PermissionUserBinding,
    ) -> Result<Vec<(&'a String, &'a PermissionUserGroup)>, String> {
        let mut groups =
            user.group_ids
                .iter()
                .map(|id| {
                    self.groups.get(id).map(|group| (id, group)).ok_or_else(|| {
                        format!("user {:?} references missing group {id:?}", user.actor)
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
        groups.sort_by(|(left_id, left), (right_id, right)| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left_id.cmp(right_id))
        });
        Ok(groups)
    }

    fn apply_profile<'a>(
        &'a self,
        id: &str,
        resolved: &mut ResolvedAccessControl,
        selected: &mut Vec<(String, &'a PermissionProfile)>,
    ) -> Result<(), String> {
        let profile = self.profile(id)?;
        if profile.complete {
            resolved.permission = profile.permission.standalone();
            resolved.grant = profile.grant.clone().unwrap_or_default();
            resolved.complete = true;
            resolved.profile_ids.clear();
            selected.clear();
        } else {
            resolved.permission.merge_from(profile.permission.clone());
            if let Some(grant) = &profile.grant {
                resolved.grant = grant.clone();
            }
        }
        if !resolved.profile_ids.iter().any(|value| value == id) {
            resolved.profile_ids.push(id.to_owned());
        }
        selected.push((id.to_owned(), profile));
        Ok(())
    }
}

fn validate_id(kind: &str, id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(format!(
            "{kind} IDs must contain 1–128 letters, digits, '.', '_' or '-'"
        ));
    }
    Ok(())
}

fn validate_name(kind: &str, name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err(format!("{kind} name must contain 1–256 non-control bytes"));
    }
    Ok(())
}

fn permissions_conflict(a: &PermissionConfig, b: &PermissionConfig) -> bool {
    // PermissionConfig equality includes the order of command rules.
    a.merged_with(b) != b.merged_with(a)
}

fn validate_identity(
    platform: &str,
    tenant: &str,
    account: &str,
    subject: &str,
) -> Result<(), String> {
    if [platform, tenant, account, subject]
        .iter()
        .any(|value| !crate::valid_channel_id(value))
    {
        return Err("permission assignments require exact platform, tenant, account and user/conversation IDs".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog() -> AccessControlConfig {
        serde_json::from_value(json!({
            "profiles": {
                "room": {"name":"Room","permission":{"path":{"workspace":"readonly"}}},
                "developer": {"name":"Developer","permission":{"path":{"workspace":"readwrite"}},
                    "grant":{"tools":["agena.fs.read","agena.fs.write"]}},
                "personal": {"name":"Personal","permission":{"network":{"default":"deny"}}}
            },
            "groups": {"developers":{"name":"Developers","priority":10,"profile_id":"developer"}},
            "users": [{"platform":"agena.telegram","tenant":"telegram","account":"bot","actor":"alice",
                "group_ids":["developers"],"profile_id":"personal",
                "overrides":{"path":{"workspace":"readonly"}}}],
            "conversations": [{"platform":"agena.telegram","tenant":"telegram","account":"bot",
                "resource":"room-1","default_profile_id":"room"}]
        })).unwrap()
    }

    fn resolve(
        config: &AccessControlConfig,
        actor: &str,
        account: &str,
        resource: &str,
    ) -> ResolvedAccessControl {
        config
            .resolve(
                "agena.telegram",
                &ChannelIdentity {
                    tenant: "telegram".into(),
                    account: account.into(),
                },
                resource,
                actor,
                &PermissionConfig::default(),
                &ChannelGrant::default(),
            )
            .unwrap()
    }

    #[test]
    fn channel_defaults_groups_and_personal_overrides_use_existing_merge() {
        let config = catalog();
        let result = resolve(&config, "alice", "bot", "room-1");
        assert_eq!(result.group_ids, ["developers"]);
        assert_eq!(result.profile_ids, ["room", "developer", "personal"]);
        assert_eq!(
            serde_json::to_value(&result.permission).unwrap()["path"]["workspace"],
            "readonly"
        );
        assert!(result.grant.tools.contains("agena.fs.write"));
        assert_eq!(result.fingerprint.len(), 64);
        let bob = resolve(&config, "bob", "bot", "room-1");
        assert_eq!(bob.profile_ids, ["room"]);
        assert!(bob.grant.tools.is_empty());
    }

    #[test]
    fn identities_and_conversations_do_not_match_other_accounts() {
        let config = catalog();
        let other = resolve(&config, "alice", "another-bot", "room-1");
        assert!(other.permission.is_empty());
        assert!(other.group_ids.is_empty());
        assert!(other.fingerprint.is_empty());
    }

    #[test]
    fn used_profiles_and_groups_cannot_be_deleted() {
        let mut config = catalog();
        config.profiles.remove("developer");
        assert!(config.validate().unwrap_err().contains("still referenced"));
        let mut config = catalog();
        config.groups.remove("developers");
        assert!(config.validate().unwrap_err().contains("missing group"));
    }

    #[test]
    fn equal_priority_conflicts_are_rejected_and_order_is_explicit() {
        let mut config = catalog();
        config.groups.insert(
            "readers".into(),
            PermissionUserGroup {
                name: "Readers".into(),
                priority: 10,
                profile_id: Some("room".into()),
            },
        );
        config.users[0].group_ids.insert("readers".into());
        assert!(
            config
                .validate()
                .unwrap_err()
                .contains("conflicting groups")
        );
        config.groups.get_mut("readers").unwrap().priority = 5;
        assert!(config.validate().is_ok());
        assert_eq!(
            resolve(&config, "alice", "bot", "room-1").group_ids,
            ["readers", "developers"]
        );
    }

    #[test]
    fn same_priority_command_rule_order_cannot_silently_choose_a_winner() {
        let deny_last: PermissionConfig = serde_json::from_str(
            r#"{"tools":{"rules":{"agena.shell.exec":{"git status":"allow","git *":"deny"}}}}"#,
        )
        .unwrap();
        let allow_last: PermissionConfig = serde_json::from_str(
            r#"{"tools":{"rules":{"agena.shell.exec":{"git *":"deny","git status":"allow"}}}}"#,
        )
        .unwrap();
        assert!(permissions_conflict(&deny_last, &allow_last));
        let read: PermissionConfig =
            serde_json::from_value(json!({"tools":{"names":{"agena.fs.read":"allow"}}})).unwrap();
        let write: PermissionConfig =
            serde_json::from_value(json!({"tools":{"names":{"agena.fs.write":"deny"}}})).unwrap();
        assert!(!permissions_conflict(&read, &write));
    }

    #[test]
    fn fingerprints_follow_used_policy_changes_but_not_unrelated_edits() {
        let mut config = catalog();
        let original = resolve(&config, "alice", "bot", "room-1").fingerprint;
        config.profiles.insert(
            "unused".into(),
            PermissionProfile {
                name: "Unused".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            original,
            resolve(&config, "alice", "bot", "room-1").fingerprint
        );
        config.profiles.get_mut("developer").unwrap().revision += 1;
        assert_ne!(
            original,
            resolve(&config, "alice", "bot", "room-1").fingerprint
        );
    }

    #[test]
    fn a_complete_profile_replaces_earlier_rule_tables_and_grants_for_this_room() {
        let mut config = catalog();
        let earlier = config.profiles.get_mut("room").unwrap();
        earlier.permission =
            serde_json::from_value(json!({"path":{"rules":{"/room/docs":"readonly"}}})).unwrap();
        earlier.grant = Some(serde_json::from_value(json!({"tools":["agena.fs.read"]})).unwrap());
        config.profiles.insert(
            "chat-only".into(),
            PermissionProfile {
                name: "Chat only".into(),
                complete: true,
                ..Default::default()
            },
        );
        config.users.push(
            serde_json::from_value(json!({
                "platform":"agena.telegram", "tenant":"telegram", "account":"bot", "actor":"alice",
                "resource":"room-1", "profile_id":"chat-only"
            }))
            .unwrap(),
        );
        let room = resolve(&config, "alice", "bot", "room-1");
        assert!(room.complete);
        assert_eq!(room.profile_ids, ["chat-only"]);
        assert!(room.grant.tools.is_empty());
        assert_eq!(room.permission, PermissionConfig::default().standalone());
        let elsewhere = resolve(&config, "alice", "bot", "room-2");
        assert!(!elsewhere.complete);
        assert!(elsewhere.grant.tools.contains("agena.fs.write"));
        assert_ne!(room.fingerprint, elsewhere.fingerprint);
    }

    #[test]
    fn fingerprints_are_independent_of_base_permissions() {
        let config = catalog();
        let identity = ChannelIdentity {
            tenant: "telegram".into(),
            account: "bot".into(),
        };
        let result = config
            .resolve(
                "agena.telegram",
                &identity,
                "room-1",
                "alice",
                &PermissionConfig::global_default(),
                &ChannelGrant::default(),
            )
            .unwrap();
        assert_eq!(
            result.fingerprint,
            resolve(&config, "alice", "bot", "room-1").fingerprint
        );
    }

    #[test]
    fn duplicate_or_wildcard_identities_fail_closed() {
        let mut config = catalog();
        config.users.push(config.users[0].clone());
        assert!(config.validate().unwrap_err().contains("duplicate"));
        let mut config = catalog();
        config.conversations[0].resource = "*".into();
        assert!(config.validate().is_err());
    }
}
