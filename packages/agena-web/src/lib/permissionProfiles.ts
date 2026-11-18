import type { JsonValue } from '@/types/json'
import type { PermissionConfig } from './permissionConfig'

export type ChannelProfileGrant = {
  tools: string[]
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
  grant?: ChannelProfileGrant
}
export type PermissionGroup = { name: string; priority: number; profile_id?: string }
export type AccessIdentity = { platform: string; tenant: string; account: string }
export type PermissionUser = AccessIdentity & {
  actor: string
  group_ids: string[]
  profile_id?: string
  overrides: PermissionConfig
}
export type PermissionConversation = AccessIdentity & { resource: string; default_profile_id: string }
export type PermissionCatalog = {
  profiles: Record<string, PermissionProfile>
  groups: Record<string, PermissionGroup>
  users: PermissionUser[]
  conversations: PermissionConversation[]
}
export type PermissionCatalogSection = keyof PermissionCatalog

export function emptyPermissionCatalog(): PermissionCatalog {
  return { profiles: {}, groups: {}, users: [], conversations: [] }
}

function isObject(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}

/** Preserve all saved fields. The server remains the authority for catalog validation. */
export function readPermissionCatalog(value: JsonValue | undefined): PermissionCatalog {
  if (value == null) return emptyPermissionCatalog()
  if (!isObject(value)) throw new Error('Invalid access_control catalog')
  if (value.profiles != null && !isObject(value.profiles)) throw new Error('Invalid access_control.profiles')
  if (value.groups != null && !isObject(value.groups)) throw new Error('Invalid access_control.groups')
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
  const groups = Object.fromEntries(
    Object.entries(value.groups ?? {}).map(([id, entry]) => [id, objectWithDefaults(entry, { name: '', priority: 0 })]),
  )
  const users = ((value.users ?? []) as unknown[]).map((entry) => {
    const user = objectWithDefaults(entry, {
      platform: '',
      tenant: '',
      account: '',
      actor: '',
      group_ids: [],
      overrides: {},
    })
    if (!Array.isArray(user.group_ids)) throw new Error('Invalid access_control user groups')
    return user
  })
  const conversations = ((value.conversations ?? []) as unknown[]).map((entry) =>
    objectWithDefaults(entry, { platform: '', tenant: '', account: '', resource: '', default_profile_id: '' }),
  )
  return JSON.parse(JSON.stringify({ ...value, profiles, groups, users, conversations })) as PermissionCatalog
}

export function permissionIdentityKey(identity: AccessIdentity, subject: string): string {
  return JSON.stringify([identity.platform, identity.tenant, identity.account, subject])
}

export function permissionProfileReferences(catalog: PermissionCatalog, id: string): string[] {
  return [
    ...Object.entries(catalog.groups)
      .filter(([, group]) => group.profile_id === id)
      .map(([key]) => key),
    ...catalog.users.filter((user) => user.profile_id === id).map((user) => permissionIdentityKey(user, user.actor)),
    ...catalog.conversations
      .filter((conversation) => conversation.default_profile_id === id)
      .map((conversation) => permissionIdentityKey(conversation, conversation.resource)),
  ]
}

export function permissionGroupReferences(catalog: PermissionCatalog, id: string): string[] {
  return catalog.users
    .filter((user) => user.group_ids.includes(id))
    .map((user) => permissionIdentityKey(user, user.actor))
}

/** Command-rule insertion order is meaningful, so retain it when comparing saved drafts. */
export function permissionCatalogSnapshot(catalog: PermissionCatalog): string {
  return JSON.stringify(catalog)
}

export function permissionGrantFromFields(
  tools: string,
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
    tools: lines(tools),
    network_hosts: lines(hosts),
    mcp_tools: Object.fromEntries(Object.entries(mcp).map(([key, value]) => [key, [...new Set(value as string[])]])),
    private_memory: privateMemory,
    source_workspace: sourceWorkspace,
  }
}
