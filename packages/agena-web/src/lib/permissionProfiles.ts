import type { JsonValue } from '@/types/json'
import { settingsText as st } from '@/i18n/settingsText'
import type { PermissionConfig } from './permissionConfig'
import type { RuntimeSettingReadResponse } from './runtimeSettings'

export const DEFAULT_PERMISSION_PROFILE_ID = '@default'

export type ChannelProfileGrant = {
  mcp_tools: Record<string, string[]>
  private_memory: boolean
  source_workspace: boolean
  network_hosts: string[]
}

export type PermissionProfile = {
  name: string
  description: string
  revision: number
  permission: PermissionConfig
  complete?: boolean
  grant?: ChannelProfileGrant
}
export type AccessIdentity = { platform: string; tenant: string; account: string }
export type ChannelPermissionTarget = AccessIdentity & { resource: string; actor?: string }
export type PermissionUser = AccessIdentity & {
  actor: string
  resource: string
  profile_id: string
}
export type PermissionConversation = AccessIdentity & { resource: string; default_profile_id: string }
export type PermissionCatalog = {
  profiles: Record<string, PermissionProfile>
  users: PermissionUser[]
  conversations: PermissionConversation[]
}

export function emptyPermissionCatalog(): PermissionCatalog {
  return { profiles: {}, users: [], conversations: [] }
}

function isObject(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}

/** Preserve all saved fields. The server remains the authority for catalog validation. */
export function readPermissionCatalog(value: JsonValue | undefined): PermissionCatalog {
  if (value == null) return emptyPermissionCatalog()
  if (!isObject(value)) throw new Error('Invalid access_control catalog')
  if (value.profiles != null && !isObject(value.profiles)) throw new Error('Invalid access_control.profiles')
  if (value.users != null && !Array.isArray(value.users)) throw new Error('Invalid access_control.users')
  if (value.conversations != null && !Array.isArray(value.conversations))
    throw new Error('Invalid access_control.conversations')
  const objectWithDefaults = (entry: unknown, defaults: Record<string, unknown>) => {
    if (!isObject(entry)) throw new Error('Invalid access_control entry')
    return { ...defaults, ...entry }
  }
  const profiles = Object.fromEntries(
    Object.entries(value.profiles ?? {}).map(([id, entry]) => [
      id,
      objectWithDefaults(entry, { name: '', description: '', revision: 1, permission: {} }),
    ]),
  )
  const users = ((value.users ?? []) as unknown[]).map((entry) => {
    const user = objectWithDefaults(entry, {
      platform: '',
      tenant: '',
      account: '',
      actor: '',
      resource: '',
      profile_id: '',
    })
    if (typeof user.resource !== 'string' || !user.resource || typeof user.profile_id !== 'string' || !user.profile_id)
      throw new Error('Member assignments require a conversation and permission profile.')
    return user
  })
  const conversations = ((value.conversations ?? []) as unknown[]).map((entry) =>
    objectWithDefaults(entry, { platform: '', tenant: '', account: '', resource: '', default_profile_id: '' }),
  )
  return JSON.parse(JSON.stringify({ profiles, users, conversations })) as PermissionCatalog
}

export function permissionProfilesForLayer(
  layer: RuntimeSettingReadResponse | undefined,
): PermissionCatalog['profiles'] {
  return {
    ...readPermissionCatalog(layer?.default_value).profiles,
    ...readPermissionCatalog(layer?.value).profiles,
  }
}

export function permissionProfileDisplayName(id: string, profile: PermissionProfile): string {
  return id === DEFAULT_PERMISSION_PROFILE_ID && profile.name === 'Default permissions'
    ? st('Default permissions')
    : profile.name || id
}

export function permissionIdentityKey(identity: AccessIdentity, subject: string): string {
  return JSON.stringify([identity.platform, identity.tenant, identity.account, subject])
}

export function permissionUserKey(user: PermissionUser): string {
  return JSON.stringify([user.platform, user.tenant, user.account, user.actor, user.resource])
}

export function channelPermissionSettings(
  { platform, tenant, account, resource, actor }: ChannelPermissionTarget,
  source?: number,
) {
  const assignment =
    actor === undefined ? [platform, tenant, account, resource] : [platform, tenant, account, resource, actor]
  return {
    path: '/settings/permissions',
    query: {
      view: 'assignments',
      scope: 'global',
      assignment: JSON.stringify(assignment),
      ...(source != null ? { channelSource: String(source) } : {}),
    },
  }
}

export function readChannelPermissionTarget(value: unknown): ChannelPermissionTarget | null {
  if (typeof value !== 'string') return null
  try {
    const parts: unknown = JSON.parse(value)
    if (
      !Array.isArray(parts) ||
      (parts.length !== 4 && parts.length !== 5) ||
      parts.some((part) => typeof part !== 'string' || !part || part !== part.trim())
    )
      return null
    const [platform, tenant, account, resource, actor] = parts as string[]
    return {
      platform: platform!,
      tenant: tenant!,
      account: account!,
      resource: resource!,
      ...(actor === undefined ? {} : { actor }),
    }
  } catch {
    return null
  }
}

export function permissionProfileReferences(catalog: PermissionCatalog, id: string): string[] {
  return [
    ...catalog.users.filter((user) => user.profile_id === id).map(permissionUserKey),
    ...catalog.conversations
      .filter((conversation) => conversation.default_profile_id === id)
      .map((conversation) => permissionIdentityKey(conversation, conversation.resource)),
  ]
}

/** Command-rule insertion order is meaningful, so retain it when comparing saved drafts. */
export function permissionCatalogSnapshot(catalog: PermissionCatalog): string {
  return JSON.stringify(catalog)
}

export function permissionGrantFromFields(
  hosts: string,
  mcp: unknown,
  privateMemory: boolean,
  sourceWorkspace: boolean,
): ChannelProfileGrant | null {
  if (
    !isObject(mcp) ||
    Object.values(mcp).some(
      (value) => !Array.isArray(value) || value.some((item) => typeof item !== 'string' || !item.trim()),
    )
  )
    return null
  const lines = (value: string) => [
    ...new Set(
      value
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter(Boolean),
    ),
  ]
  return {
    network_hosts: lines(hosts),
    mcp_tools: Object.fromEntries(Object.entries(mcp).map(([key, value]) => [key, [...new Set(value as string[])]])),
    private_memory: privateMemory,
    source_workspace: sourceWorkspace,
  }
}
