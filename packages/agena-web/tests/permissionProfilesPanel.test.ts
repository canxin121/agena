import { setupSettingsDraftGuard } from './helpers/setupSettingsDraftGuard'
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'
import * as permissionConfig from '../src/lib/permissionConfig'
import * as permissionProfiles from '../src/lib/permissionProfiles'
import { buildSettingsSubpages } from '../src/components/settings/settingsNavigationCatalog'
import { buildSettingsNavigationCategories } from '../src/components/settings/settingsInformationArchitecture'
import { buildSettingsScopes } from '../src/components/settings/settingsScopeCatalog'

const source = readFileSync(new URL('../src/components/settings/PermissionProfilesPanel.vue', import.meta.url), 'utf8')
const { descriptor } = parse(source)
const componentCode = transformSync(compileScript(descriptor, { id: 'permission-profiles-test' }).content, {
  loader: 'ts',
  format: 'cjs',
}).code
const transportCode = transformSync(readFileSync(new URL('../src/lib/runtimeSettings.ts', import.meta.url), 'utf8'), {
  loader: 'ts',
  format: 'cjs',
}).code
const scopes: vue.EffectScope[] = []
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
})
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value))

async function editor(
  initial = permissionProfiles.emptyPermissionCatalog(),
  defaults?: permissionProfiles.PermissionCatalog,
  assignment?: string,
) {
  let state = clone(initial)
  let revision = 1
  let failure = ''
  let raceNextWrite = false
  const route = vue.reactive({ query: { assignment, channelSource: '7' } })
  const writes: Array<{ path: string; value: permissionProfiles.PermissionCatalog; expected_revision?: string }> = []
  const requests: Array<{ url: string; method: string; body?: Record<string, unknown> }> = []
  const api = {
    async apiJson(url: string, init?: RequestInit) {
      const body = init?.body ? JSON.parse(String(init.body)) : undefined
      requests.push({ url, method: init?.method || 'GET', body })
      if (!init?.method) return { value: clone(state), default_value: defaults, revision: 'r' + revision }
      if (raceNextWrite) {
        revision += 1
        raceNextWrite = false
      }
      const expected =
        body?.expected_revision ?? new URL(url, 'https://agena.test').searchParams.get('expected_revision')
      if (expected !== 'r' + revision) throw new Error('Configuration changed: revision conflict')
      if (failure) throw new Error(failure)
      if (init.method === 'PUT') {
        state = clone(body.value)
        writes.push(clone(body))
      }
      revision += 1
      return {
        current: clone(state),
        changed: true,
        before_revision: 'r' + (revision - 1),
        after_revision: 'r' + revision,
      }
    },
  }
  const transportModules: Record<string, unknown> = {
    '@/lib/api': api,
    './modelConfigurationApi': { notifyModelConfigurationChanged() {} },
    './resourceSync': { captureResourceObservation: () => ({ scope: 1 }), invalidateResourcePrefix() {} },
    './runtimeSettingReads': { retireRuntimeSettingReads() {}, readBatchedRuntimeSettingSources() {} },
  }
  const transport = { exports: {} as Record<string, (...args: any[]) => Promise<unknown>> }
  new Function('require', 'module', 'exports', transportCode)(
    (id: string) => {
      if (!(id in transportModules)) throw new Error('Unexpected transport import: ' + id)
      return transportModules[id]
    },
    transport,
    transport.exports,
  )
  const modules: Record<string, unknown> = {
    vue: { ...vue, onMounted() {}, onBeforeUnmount() {} },
    'vue-i18n': { useI18n: () => ({ t: (key: string) => key }) },
    'vue-router': { onBeforeRouteLeave() {}, onBeforeRouteUpdate() {}, useRoute: () => route },
    '@remixicon/vue': {},
    '@/lib/appConfirm': { confirmAction: async () => true },
    '@/lib/permissionConfig': permissionConfig,
    '@/lib/permissionProfiles': permissionProfiles,
    '@/lib/runtimeSettings': transport.exports,
    '@/i18n/settingsText': {
      settingsText: (key: string, params: Record<string, unknown> = {}) =>
        key.replace(/\{(\w+)\}/g, (_, name: string) => String(params[name] ?? '{' + name + '}')),
    },
  }
  const component = { exports: {} as { default?: { setup: (props: unknown, context: unknown) => any } } }
  setupSettingsDraftGuard(modules)
  new Function('require', 'module', 'exports', componentCode)(
    (id: string) => {
      if (id.endsWith('.vue')) return {}
      if (!(id in modules)) throw new Error('Unexpected component import: ' + id)
      return modules[id]
    },
    component,
    component.exports,
  )
  const scope = vue.effectScope()
  scopes.push(scope)
  const panel = scope.run(() =>
    component.exports.default!.setup({ assignments: assignment !== undefined }, { expose() {} }),
  )!
  await panel.load()
  return {
    panel,
    writes,
    requests,
    route,
    transport: transport.exports,
    current: () => clone(state),
    fail(message: string) {
      failure = message
    },
    race() {
      raceNextWrite = true
    },
  }
}

describe('permission configuration management', () => {
  test('opens the chat directory by default and saves through the same revision guard', async () => {
    const catalog = permissionProfiles.emptyPermissionCatalog()
    const room = { platform: 'agena.telegram', tenant: 'telegram', account: 'bot', resource: '-1007' }
    catalog.profiles.reader = { name: 'Reader', description: '', revision: 1, permission: {} }
    const { panel, writes, race, current } = await editor(catalog, undefined, '')
    const next = { ...catalog, conversations: [{ ...room, default_profile_id: 'reader' }] }
    race()
    await expect(panel.saveChannelAssignments(next)).rejects.toThrow('revision conflict')
    expect(current()).toEqual(catalog)
    expect(writes).toHaveLength(0)
    await panel.load()
    await panel.saveChannelAssignments(next)
    expect(writes[0]?.expected_revision).toBe('r2')
    expect(current()).toEqual(next)
  })

  test('passes observed room and member links to the conversation directory', async () => {
    const room = { platform: 'custom.chat', tenant: 'team', account: 'bot', resource: 'channel/研发?&=2' }
    const actor = 'person-1'
    const member = permissionProfiles.channelPermissionSettings({ ...room, actor }, 7).query.assignment
    const { panel, route, writes } = await editor(permissionProfiles.emptyPermissionCatalog(), undefined, member)
    expect(panel.channelTarget.value).toEqual({ ...room, actor })
    expect(panel.channelSource.value).toBe(7)
    expect(panel.draft.value).toBeNull()

    route.query.assignment = permissionProfiles.channelPermissionSettings(room, 7).query.assignment
    await vue.nextTick()
    expect(panel.channelTarget.value).toEqual(room)
    expect(panel.draft.value).toBeNull()
    expect(writes).toHaveLength(0)
  })

  test('the default profile is present and editable before it has an authored record', async () => {
    const id = permissionProfiles.DEFAULT_PERMISSION_PROFILE_ID
    const defaults = permissionProfiles.emptyPermissionCatalog()
    defaults.profiles[id] = {
      name: 'Default permissions',
      complete: true,
      description: '',
      revision: 1,
      permission: {},
    }
    const { panel, current } = await editor(permissionProfiles.emptyPermissionCatalog(), defaults)
    expect(panel.rows.value.map((row: { id: string }) => row.id)).toEqual([id])
    expect(current().profiles).toEqual({})
    await panel.selectEntry(id)
    panel.draft.value.permission.tools.default = 'ask'
    await panel.save()
    expect(current().profiles[id]).toMatchObject({
      revision: 2,
      complete: true,
      permission: { tools: { default: 'ask' } },
    })
  })

  test('is reachable only in global settings', () => {
    expect(buildSettingsSubpages('permissions').some((page) => page.id === 'profiles')).toBe(true)
    expect(buildSettingsNavigationCategories().find((category) => category.id === 'security')?.children).toEqual(
      expect.arrayContaining([expect.objectContaining({ section: 'permissions', view: 'profiles' })]),
    )
    expect(buildSettingsScopes('permissions', 'profiles').map((scope) => scope.id)).toEqual(['global'])
  })

  test('creates and copies profiles with revision guards', async () => {
    const { panel, writes, requests, current } = await editor()
    await panel.addEntry()
    Object.assign(panel.draft.value, {
      id: 'readers',
      name: 'Readers',
      grantEnabled: true,
      mcp: [{ server: 'docs', tools: 'search' }],
      permission: {
        path: { workspace: 'readonly', external: 'deny' },
        tools: { default: 'ask', names: { 'fs.read': 'allow' } },
      },
    })
    await panel.save()
    expect(panel.error.value).toBe('')
    expect(current().profiles.readers?.grant).toEqual({
      mcp_tools: { docs: ['search'] },
      network_hosts: [],
      private_memory: false,
      source_workspace: false,
    })
    expect(current().profiles.readers?.permission.tools?.names).toEqual({ 'fs.read': 'allow' })
    expect(current().profiles.readers?.revision).toBe(1)
    expect(current().profiles.readers).toMatchObject({ complete: true, permission: { network: { default: 'deny' } } })
    await panel.addEntry(true)
    panel.draft.value.id = 'researchers'
    panel.draft.value.name = 'Researchers'
    await panel.save()
    expect(current().profiles.researchers?.permission).toEqual(current().profiles.readers?.permission)

    expect(writes.map((write) => write.expected_revision)).toEqual(['r1', 'r2'])
    expect(requests.every((request) => request.url.startsWith('/api/v1/settings/layers/global'))).toBe(true)
    expect(panel.dirty.value).toBe(false)
  })

  test('requires removing member and conversation bindings before deleting a profile', async () => {
    const catalog = permissionProfiles.emptyPermissionCatalog()
    catalog.profiles.reader = { name: 'Reader', description: '', revision: 1, permission: {} }
    const room = { platform: 'agena.telegram', tenant: 'telegram', account: 'bot', resource: '-1007' }
    catalog.users.push({ ...room, actor: 'one', profile_id: 'reader' })
    catalog.conversations.push({ ...room, default_profile_id: 'reader' })
    const { panel, writes, current } = await editor(catalog)
    await panel.selectEntry('reader')
    await panel.removeEntry()
    expect(panel.error.value).toContain('in use')
    expect(writes).toHaveLength(0)
    await panel.saveChannelAssignments({ ...catalog, users: [], conversations: [] })
    await panel.removeEntry()
    expect(current()).toEqual(permissionProfiles.emptyPermissionCatalog())
  })

  test('retains unsaved edits on server rejection and rejects an intervening write atomically', async () => {
    const catalog = permissionProfiles.emptyPermissionCatalog()
    catalog.profiles.reader = { name: 'Reader', description: '', revision: 1, permission: {} }
    const harness = await editor(catalog)
    await harness.panel.selectEntry('reader')
    harness.panel.draft.value.name = 'Changed'
    harness.fail('Invalid permission profile')
    await harness.panel.save()
    expect(harness.panel.error.value).toContain('Invalid permission profile')
    expect(harness.panel.draft.value.name).toBe('Changed')
    expect(harness.panel.dirty.value).toBe(true)
    harness.fail('')
    harness.race()
    await harness.panel.save()
    expect(harness.panel.error.value).toContain('revision conflict')
    expect(harness.current().profiles.reader?.name).toBe('Reader')
    expect(harness.panel.draft.value.name).toBe('Changed')
    expect(harness.writes).toHaveLength(0)
  })

  test('reads optional profile fields with the persisted defaults', async () => {
    const catalog = permissionProfiles.readPermissionCatalog({ profiles: { reader: { name: 'Reader' } } })
    const { panel } = await editor(catalog)
    await panel.selectEntry('reader')
    expect(panel.draft.value.permission).toEqual({})
    expect(panel.draft.value.description).toBe('')
    expect(catalog.profiles.reader?.revision).toBe(1)
  })

  test('settings helper forwards revision guards for PATCH and DELETE too', async () => {
    const { transport, requests } = await editor()
    await transport.patchRuntimeSettings!('access_control', {}, { expected_revision: 'r1' })
    await transport.deleteRuntimeSetting!('access_control', { expected_revision: 'r2' }, 'global')
    expect(requests.find((request) => request.method === 'PATCH')?.body?.expected_revision).toBe('r1')
    const deletion = requests.find((request) => request.method === 'DELETE')
    expect(new URL(deletion!.url, 'https://agena.test').searchParams.get('expected_revision')).toBe('r2')
  })
})
