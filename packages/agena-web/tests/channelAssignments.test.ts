import { expect, test } from 'bun:test'
import {
  applyChannelAssignments,
  channelAssignmentRooms,
  mergeChannelParticipants,
  roomPermissionBindings,
} from '../src/components/settings/channelAssignments'
import type { PlatformInventory } from '../src/generated/agenaState'
import { emptyPermissionCatalog } from '../src/lib/permissionProfiles'

const inventory: PlatformInventory = {
  platforms: [
    {
      platform: 'agena.telegram',
      title: 'Telegram',
      conversation_count: 3,
      conversation_counts: { group: 3, direct: 0, broadcast: 0 },
      accounts: [
        {
          binding_id: 'one',
          tenant: 'telegram',
          account: 'work',
          state: 'enabled',
          selector: '-100123',
          conversations: [
            { resource: '-100123', kind: 'group', room_title: 'Engineering', session_id: 10, thread: 'release' },
            { resource: '-100123', kind: 'group', room_title: 'Engineering', session_id: 12, thread: 'design' },
          ],
        },
        {
          binding_id: 'two',
          tenant: 'telegram',
          account: 'work',
          state: 'paused',
          selector: '-100123',
          conversations: [{ resource: '-100123', kind: 'group' }],
        },
        {
          binding_id: 'three',
          tenant: 'telegram',
          account: 'personal',
          state: 'enabled',
          selector: '-100123',
          conversations: [{ resource: '-100123', kind: 'group', room_title: 'Personal' }],
        },
      ],
    },
  ],
}

test('selects configured rooms and combines topic bindings only within the same account', () => {
  const [work, personal] = channelAssignmentRooms(inventory)
  expect(work?.title).toBe('Engineering')
  expect(work?.locations).toEqual([
    { bindingId: 'one', kind: 'group' },
    { bindingId: 'two', kind: 'group' },
  ])
  expect(personal?.identity.account).toBe('personal')
  expect(personal?.locations).toEqual([{ bindingId: 'three', kind: 'group' }])
  expect(
    mergeChannelParticipants(
      [{ actor: '7', display_name: 'Earlier', first_seen_at_ms: 100, last_seen_at_ms: 150 }],
      [{ actor: '7', display_name: 'Current', first_seen_at_ms: 200, last_seen_at_ms: 250 }],
    ),
  ).toEqual([{ actor: '7', display_name: 'Current', first_seen_at_ms: 100, last_seen_at_ms: 250 }])
})

test('bulk edits keep individual overrides and account-wide groups outside the selected room draft', () => {
  const [room, personal] = channelAssignmentRooms(inventory)
  const scoped = room!.identity
  const catalog = emptyPermissionCatalog()
  const accountBinding = {
    platform: scoped.platform,
    tenant: scoped.tenant,
    account: scoped.account,
    actor: '7',
    group_ids: ['account-group'],
    overrides: {},
  }
  const personalBinding = { ...personal!.identity, actor: '7', group_ids: ['other-account'], overrides: {} }
  catalog.users = [
    accountBinding,
    {
      ...scoped,
      actor: '7',
      group_ids: ['readers'],
      profile_id: 'member-profile',
      overrides: { tools: { names: { shell: 'deny' } } },
    },
    { ...scoped, actor: '8', group_ids: ['readers'], overrides: {} },
    personalBinding,
  ]
  catalog.conversations = [
    { ...scoped, default_profile_id: 'old' },
    { ...personal!.identity, default_profile_id: 'other' },
  ]
  const next = applyChannelAssignments(
    catalog,
    room!,
    'read',
    new Map([
      ['7', ['readers', 'reviewers']],
      ['8', []],
      ['9', ['reviewers']],
    ]),
  )
  const bindings = roomPermissionBindings(next, room!)
  expect(bindings.profile).toBe('read')
  expect(bindings.members.get('7')).toEqual({ ...catalog.users[1]!, group_ids: ['readers', 'reviewers'] })
  expect(bindings.members.has('8')).toBe(false)
  expect(bindings.members.get('9')).toEqual({ ...scoped, actor: '9', group_ids: ['reviewers'], overrides: {} })
  expect(bindings.accountMembers.get('7')).toEqual(accountBinding)
  expect(next.users).toContain(personalBinding)
  expect(applyChannelAssignments(next, room!, '', new Map()).conversations).toEqual([catalog.conversations[1]!])
})
