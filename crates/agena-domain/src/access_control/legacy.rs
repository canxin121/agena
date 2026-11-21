//! Upgrade saved permission assignments to exact room/member profiles.
use super::*;

pub(super) fn deserialize_profile_grant<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ChannelGrant>, D::Error> {
    Ok(
        Option::<ChannelGrant>::deserialize(deserializer)?.map(|mut grant| {
            grant.tools.clear();
            grant
        }),
    )
}

pub(super) fn serialize_profile_grant<S: serde::Serializer>(
    grant: &Option<ChannelGrant>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let Some(grant) = grant else {
        return serializer.serialize_none();
    };
    use serde::ser::SerializeStruct;
    let mut value = serializer.serialize_struct("ChannelResourceScope", 4)?;
    value.serialize_field("mcp_tools", &grant.mcp_tools)?;
    value.serialize_field("private_memory", &grant.private_memory)?;
    value.serialize_field("source_workspace", &grant.source_workspace)?;
    value.serialize_field("network_hosts", &grant.network_hosts)?;
    value.end()
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct StoredAccessControl {
    profiles: BTreeMap<String, PermissionProfile>,
    conversations: Vec<PermissionConversationBinding>,
    users: Vec<StoredUser>,
    groups: BTreeMap<String, StoredGroup>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct StoredGroup {
    name: String,
    priority: i32,
    profile_id: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct StoredUser {
    platform: String,
    tenant: String,
    account: String,
    actor: String,
    resource: Option<String>,
    profile_id: Option<String>,
    group_ids: Option<BTreeSet<String>>,
    overrides: PermissionConfig,
}

type Account<'a> = (&'a str, &'a str, &'a str);

impl StoredUser {
    fn account(&self) -> Account<'_> {
        (&self.platform, &self.tenant, &self.account)
    }

    fn binding(&self, resource: &str, profile_id: String) -> PermissionUserBinding {
        PermissionUserBinding {
            platform: self.platform.clone(),
            tenant: self.tenant.clone(),
            account: self.account.clone(),
            actor: self.actor.clone(),
            resource: resource.into(),
            profile_id,
        }
    }
}

impl TryFrom<StoredAccessControl> for AccessControlConfig {
    type Error = String;

    fn try_from(stored: StoredAccessControl) -> Result<Self, String> {
        let mut config = AccessControlConfig {
            profiles: stored.profiles,
            ..Default::default()
        };
        let mut rooms: BTreeMap<Account<'_>, BTreeMap<&str, Option<&str>>> = BTreeMap::new();
        for room in &stored.conversations {
            rooms
                .entry((&room.platform, &room.tenant, &room.account))
                .or_default()
                .insert(&room.resource, Some(&room.default_profile_id));
        }
        let mut scope_users = BTreeMap::new();
        for user in &stored.users {
            let resource = user.resource.as_deref();
            validate_identity(&user.platform, &user.tenant, &user.account, &user.actor)?;
            if scope_users
                .insert((user.account(), user.actor.as_str(), resource), user)
                .is_some()
            {
                return Err(format!(
                    "duplicate access_control user {:?} in {resource:?}",
                    user.actor
                ));
            }
            if let Some(resource) = resource {
                rooms
                    .entry(user.account())
                    .or_default()
                    .entry(resource)
                    .or_default();
            }
        }
        for user in &stored.users {
            if let Some(resource) = &user.resource {
                let room_profile = rooms[&user.account()][resource.as_str()];
                let account_user = scope_users
                    .get(&(user.account(), user.actor.as_str(), None))
                    .copied();
                if account_user.is_none() && user.group_ids.is_none() && user.overrides.is_empty() {
                    if let Some(id) = &user.profile_id {
                        config.users.push(user.binding(resource, id.clone()));
                    }
                } else if let Some(id) = migrate_profile(
                    &mut config,
                    &stored.groups,
                    room_profile,
                    account_user,
                    Some(user),
                )? {
                    config.users.push(user.binding(resource, id));
                }
            } else if let Some(account_rooms) = rooms.get(&user.account()) {
                for (&resource, &profile) in account_rooms {
                    if !scope_users.contains_key(&(
                        user.account(),
                        user.actor.as_str(),
                        Some(resource),
                    )) && let Some(id) =
                        migrate_profile(&mut config, &stored.groups, profile, Some(user), None)?
                    {
                        if config.users.len() >= 10000 {
                            return Err(
                                "legacy access_control expands beyond the member assignment limit"
                                    .into(),
                            );
                        }
                        config.users.push(user.binding(resource, id));
                    }
                }
            }
        }
        config.conversations = stored.conversations;
        Ok(config)
    }
}

fn migrate_profile(
    config: &mut AccessControlConfig,
    groups: &BTreeMap<String, StoredGroup>,
    room_profile: Option<&str>,
    account_user: Option<&StoredUser>,
    member: Option<&StoredUser>,
) -> Result<Option<String>, String> {
    let mut effective = PermissionProfile::default();
    let mut last_profile = room_profile;
    if let Some(id) = room_profile {
        apply_profile(&mut effective, config.profile(id)?);
    }
    let mut assigned = false;
    for user in [account_user, member].into_iter().flatten() {
        let mut ordered = Vec::new();
        for id in user.group_ids.iter().flatten() {
            let group = groups
                .get(id)
                .ok_or_else(|| format!("missing legacy permission group {id:?}"))?;
            validate_name("group", &group.name)?;
            if let Some(profile) = group.profile_id.as_deref() {
                ordered.push((group.priority, id, profile));
            }
        }
        ordered.sort_unstable();
        for id in ordered
            .into_iter()
            .map(|(_, _, profile)| profile)
            .chain(user.profile_id.as_deref())
        {
            apply_profile(&mut effective, config.profile(id)?);
            last_profile = Some(id);
            assigned = true;
        }
        effective.permission.merge_from(user.overrides.clone());
        assigned |= !user.overrides.is_empty();
    }
    if !assigned {
        return Ok(None);
    }
    if let Some(id) = last_profile {
        let mut original = PermissionProfile::default();
        apply_profile(&mut original, config.profile(id)?);
        if original == effective {
            return Ok(Some(id.into()));
        }
    }
    let payload = serde_json::to_vec(&effective).map_err(|error| error.to_string())?;
    let digest = digest(&payload);
    let id = format!("migrated-{digest}");
    effective.name = format!("Migrated channel permissions ({})", &digest[..8]);
    let saved = config
        .profiles
        .entry(id.clone())
        .or_insert_with(|| effective.clone());
    if saved != &effective {
        return Err(format!("conflicting migrated permission profile {id:?}"));
    }
    Ok(Some(id))
}

fn apply_profile(effective: &mut PermissionProfile, profile: &PermissionProfile) {
    if profile.complete {
        effective.permission = profile.permission.standalone();
        effective.grant = profile.grant.clone();
        effective.complete = true;
    } else {
        effective.permission.merge_from(profile.permission.clone());
        if let Some(grant) = &profile.grant {
            effective.grant = Some(grant.clone());
        }
    }
}
