import type { ChannelParticipant, ConversationKind, PlatformInventory } from '@/generated/agenaState'
import {
  permissionIdentityKey,
  type ChannelPermissionTarget,
  type PermissionCatalog,
  type PermissionUser,
} from '@/lib/permissionProfiles'

type RoomIdentity = Omit<ChannelPermissionTarget, 'actor'>
export type ChannelRoomLocation = { bindingId: string; kind: ConversationKind }
export type ChannelRoom = {
  key: string
  identity: RoomIdentity
  kind: ConversationKind
  platformTitle: string
  title?: string
  locations: ChannelRoomLocation[]
}

/** Bindings can overlap for a room, for example when separate topics are connected. */
export function channelAssignmentRooms(inventory: PlatformInventory): ChannelRoom[] {
  const rooms = new Map<string, ChannelRoom>()
  for (const platform of inventory.platforms) {
    for (const account of platform.accounts) {
      for (const conversation of account.conversations) {
        const identity = {
          platform: platform.platform,
          tenant: account.tenant,
          account: account.account,
          resource: conversation.resource,
        }
        const key = permissionIdentityKey(identity, identity.resource)
        let room = rooms.get(key)
        if (!room) {
          room = {
            key,
            identity,
            kind: conversation.kind,
            platformTitle: platform.title || platform.platform,
            locations: [],
          }
          rooms.set(key, room)
        }
        room.title ||= conversation.room_title?.trim() || undefined
        if (
          !room.locations.some(({ bindingId, kind }) => bindingId === account.binding_id && kind === conversation.kind)
        ) {
          room.locations.push({ bindingId: account.binding_id, kind: conversation.kind })
        }
      }
    }
  }
  return [...rooms.values()].sort((a, b) =>
    (a.title || a.identity.resource).localeCompare(b.title || b.identity.resource),
  )
}

export function mergeChannelParticipants(
  current: ChannelParticipant[],
  incoming: ChannelParticipant[],
): ChannelParticipant[] {
  const merged = new Map(current.map((participant) => [participant.actor, participant]))
  for (const participant of incoming) {
    const previous = merged.get(participant.actor)
    if (!previous) {
      merged.set(participant.actor, participant)
      continue
    }
    const [earlier, later] =
      previous.last_seen_at_ms > participant.last_seen_at_ms ? [participant, previous] : [previous, participant]
    merged.set(participant.actor, {
      ...later,
      display_name: later.display_name || earlier.display_name,
      first_seen_at_ms: Math.min(earlier.first_seen_at_ms, later.first_seen_at_ms),
    })
  }
  return [...merged.values()].sort(
    (a, b) => (a.display_name || a.actor).localeCompare(b.display_name || b.actor) || a.actor.localeCompare(b.actor),
  )
}

export function roomPermissionBindings(catalog: PermissionCatalog, room: ChannelRoom) {
  const members = new Map<string, PermissionUser>()
  for (const user of catalog.users) {
    if (permissionIdentityKey(user, user.resource) === room.key) members.set(user.actor, user)
  }
  return {
    members,
    profile:
      catalog.conversations.find(
        (conversation) => permissionIdentityKey(conversation, conversation.resource) === room.key,
      )?.default_profile_id || '',
  }
}

/** An empty selection removes the assignment and restores inheritance. */
export function applyChannelAssignments(
  catalog: PermissionCatalog,
  room: ChannelRoom,
  profile: string | undefined,
  memberEdits: ReadonlyMap<string, string>,
): PermissionCatalog {
  const pending = new Map(memberEdits)
  const users = catalog.users.flatMap((user) => {
    if (permissionIdentityKey(user, user.resource) !== room.key) return [user]
    const profileId = pending.get(user.actor)
    if (profileId === undefined) return [user]
    pending.delete(user.actor)
    return profileId ? [{ ...user, profile_id: profileId }] : []
  })
  for (const [actor, profileId] of pending) {
    if (profileId) users.push({ ...room.identity, actor, profile_id: profileId })
  }
  if (profile === undefined) return { ...catalog, users }

  const conversations = catalog.conversations.filter(
    (entry) => permissionIdentityKey(entry, entry.resource) !== room.key,
  )
  if (profile) conversations.push({ ...room.identity, default_profile_id: profile })
  return { ...catalog, users, conversations }
}
