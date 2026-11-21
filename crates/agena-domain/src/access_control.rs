//! Named, instance-owned permission configurations and exact channel assignments.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{ChannelGrant, ChannelIdentity, PermissionConfig};

mod legacy;

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
#[serde(try_from = "legacy::StoredAccessControl")]
pub struct AccessControlConfig {
    pub profiles: BTreeMap<String, PermissionProfile>,
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
    /// Complete configurations use standalone execution rules. Older profiles
    /// can still supply an overlay on the source execution rules.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub complete: bool,
    /// Optional replacement for the channel's resource scope.
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "legacy::serialize_profile_grant",
        deserialize_with = "legacy::deserialize_profile_grant"
    )]
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
#[serde(deny_unknown_fields)]
pub struct PermissionUserBinding {
    pub platform: String,
    pub tenant: String,
    pub account: String,
    pub actor: String,
    pub resource: String,
    pub profile_id: String,
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
        self.profiles.is_empty() && self.users.is_empty() && self.conversations.is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.profiles.len()
            - usize::from(self.profiles.contains_key(DEFAULT_PERMISSION_PROFILE_ID))
            > 256
            || self.users.len() > 10000
            || self.conversations.len() > 4096
        {
            return Err("access_control exceeds profile/member/conversation limits".into());
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
        let mut identities = BTreeSet::new();
        for user in &self.users {
            validate_identity(&user.platform, &user.tenant, &user.account, &user.actor)?;
            validate_identity(&user.platform, &user.tenant, &user.account, &user.resource)?;
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
            self.profile(&user.profile_id)?;
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

    /// An exact member assignment selects one profile in place of the room default.
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
        let profile_id = self
            .users
            .iter()
            .find(|item| {
                item.platform == platform
                    && item.tenant == identity.tenant
                    && item.account == identity.account
                    && item.resource == resource
                    && item.actor == actor
            })
            .map(|item| &item.profile_id)
            .or_else(|| {
                self.conversations
                    .iter()
                    .find(|item| {
                        item.platform == platform
                            && item.tenant == identity.tenant
                            && item.account == identity.account
                            && item.resource == resource
                    })
                    .map(|item| &item.default_profile_id)
            });
        let Some(id) = profile_id.filter(|_| crate::valid_channel_id(actor)) else {
            return Ok(ResolvedAccessControl {
                permission: base_permission.clone(),
                grant: base_grant.clone(),
                ..Default::default()
            });
        };
        let profile = self.profile(id)?;
        let permission = if profile.complete {
            profile.permission.standalone()
        } else {
            base_permission.merged_with(&profile.permission)
        };
        let mut grant = match &profile.grant {
            Some(grant) => grant.clone(),
            None if profile.complete => ChannelGrant::default(),
            None => base_grant.clone(),
        };
        grant.tools.clear();
        // An execution admitted under the old tool allowlist must not gain
        // tools when it resumes. New messages resolve the same conversation.
        let payload = serde_json::to_vec(&("tool-permission-rules-v1", id, profile))
            .map_err(|error| format!("cannot fingerprint permission assignment: {error}"))?;
        Ok(ResolvedAccessControl {
            permission,
            complete: profile.complete,
            grant,
            profile_ids: vec![id.clone()],
            fingerprint: digest(&payload),
        })
    }

    fn profile(&self, id: &str) -> Result<&PermissionProfile, String> {
        self.profiles
            .get(id)
            .or_else(|| (id == DEFAULT_PERMISSION_PROFILE_ID).then(|| &*DEFAULT_PERMISSION_PROFILE))
            .ok_or_else(|| {
                format!("permission profile {id:?} is missing or still referenced after deletion")
            })
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
                "room": {"name":"Room","permission":{"path":{"workspace":"readonly"}},
                    "grant":{"tools":["agena.fs.read"],"source_workspace":true}},
                "member": {"name":"Member","permission":{"network":{"default":"deny"}}}
            },
            "users": [{"platform":"agena.telegram","tenant":"telegram","account":"bot","actor":"alice",
                "resource":"room-1","profile_id":"member"}],
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
    fn selects_the_exact_member_profile_or_the_room_default() {
        let config = catalog();
        let result = resolve(&config, "alice", "bot", "room-1");
        assert_eq!(result.profile_ids, ["member"]);
        assert!(result.permission.path.is_none());
        assert!(result.grant.tools.is_empty());
        assert_eq!(result.fingerprint.len(), 64);

        let bob = resolve(&config, "bob", "bot", "room-1");
        assert_eq!(bob.profile_ids, ["room"]);
        assert!(bob.grant.tools.is_empty());
        assert!(bob.grant.source_workspace);
        let serialized = serde_json::to_value(&config).unwrap();
        assert!(
            serialized["profiles"]["room"]["grant"]
                .get("tools")
                .is_none()
        );

        for (account, resource) in [("another-bot", "room-1"), ("bot", "room-2")] {
            let other = resolve(&config, "alice", account, resource);
            assert!(other.permission.is_empty());
            assert!(other.fingerprint.is_empty());
        }
    }

    #[test]
    fn used_profiles_cannot_be_deleted() {
        let mut config = catalog();
        config.profiles.remove("member");
        assert!(config.validate().unwrap_err().contains("still referenced"));
    }

    #[test]
    fn migrates_old_groups_and_account_rules_to_visible_room_assignments() {
        let config: AccessControlConfig = serde_json::from_value(json!({
            "profiles": {
                "room": {"name":"Room","permission":{"path":{"workspace":"readonly"}}},
                "developer": {"name":"Developer","permission":{"path":{"workspace":"readwrite"}},
                    "grant":{"source_workspace":true}},
                "personal": {"name":"Personal","permission":{"network":{"default":"deny"}}}
            },
            "groups": {"developers":{"name":"Developers","priority":10,"profile_id":"developer"}},
            "users": [
                {"platform":"agena.telegram","tenant":"telegram","account":"bot","actor":"alice",
                    "group_ids":["developers"]},
                {"platform":"agena.telegram","tenant":"telegram","account":"bot","resource":"room-1","actor":"alice",
                    "profile_id":"personal", "overrides":{"path":{"workspace":"readonly"}}}
            ],
            "conversations": [{"platform":"agena.telegram","tenant":"telegram","account":"bot",
                "resource":"room-1","default_profile_id":"room"}]
        })).unwrap();
        assert_eq!(config.users.len(), 1);
        let alice = resolve(&config, "alice", "bot", "room-1");
        assert_eq!(alice.profile_ids, [config.users[0].profile_id.clone()]);
        assert_eq!(
            serde_json::to_value(&alice.permission).unwrap()["path"]["workspace"],
            "readonly"
        );
        assert!(alice.grant.source_workspace);
        assert!(
            resolve(&config, "alice", "bot", "room-2")
                .profile_ids
                .is_empty()
        );
        let json = serde_json::to_value(&config).unwrap();
        assert!(json.get("groups").is_none());
        assert!(json["users"][0].get("overrides").is_none());
        assert_eq!(
            serde_json::from_value::<AccessControlConfig>(json).unwrap(),
            config
        );
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
        config.profiles.get_mut("room").unwrap().revision += 1;
        assert_eq!(
            original,
            resolve(&config, "alice", "bot", "room-1").fingerprint
        );
        config.profiles.get_mut("member").unwrap().revision += 1;
        assert_ne!(
            original,
            resolve(&config, "alice", "bot", "room-1").fingerprint
        );
    }

    #[test]
    fn a_complete_member_profile_replaces_the_room_resource_scope() {
        let mut config = catalog();
        let earlier = config.profiles.get_mut("room").unwrap();
        earlier.permission =
            serde_json::from_value(json!({"path":{"rules":{"/room/docs":"readonly"}}})).unwrap();
        config.profiles.insert(
            "private".into(),
            PermissionProfile {
                name: "Private resources".into(),
                complete: true,
                ..Default::default()
            },
        );
        config.users[0].profile_id = "private".into();
        let room = resolve(&config, "alice", "bot", "room-1");
        assert!(room.complete);
        assert_eq!(room.profile_ids, ["private"]);
        assert_eq!(room.grant, ChannelGrant::default());
        assert_eq!(room.permission, PermissionConfig::default().standalone());
        let elsewhere = resolve(&config, "alice", "bot", "room-2");
        assert!(!elsewhere.complete);
        assert!(elsewhere.grant.tools.is_empty());
        assert_ne!(room.fingerprint, elsewhere.fingerprint);
    }

    #[test]
    fn the_room_profile_applies_only_to_channel_members() {
        let config = catalog();
        for actor in ["@service", "@shared"] {
            let resolved = resolve(&config, actor, "bot", "room-1");
            assert!(resolved.profile_ids.is_empty());
            assert_eq!(resolved.grant, ChannelGrant::default());
        }
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
