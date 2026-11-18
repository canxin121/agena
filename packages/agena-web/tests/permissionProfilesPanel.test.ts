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

async function editor(initial = permissionProfiles.emptyPermissionCatalog()) {
  let state = clone(initial)
  let revision = 1
  let failure = ''
  let raceNextWrite = false
  const writes: Array<{ path: string; value: permissionProfiles.PermissionCatalog; expected_revision?: string }> = []
  const requests: Array<{ url: string; method: string; body?: Record<string, unknown> }> = []
  const api = {
    async apiJson(url: string, init?: RequestInit) {
      const body = init?.body ? JSON.parse(String(init.body)) : undefined
      requests.push({ url, method: init?.method || 'GET', body })
      if (!init?.method) return { value: clone(state), revision: 'r' + revision }
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
    './resourceSync': { captureResourceObservation: () => ({ scope: 1 }) },
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
    'vue-router': { onBeforeRouteLeave() {}, onBeforeRouteUpdate() {} },
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
  const panel = scope.run(() => component.exports.default!.setup({}, { expose() {} }))!
  await panel.load()
  return {
    panel,
    writes,
    requests,
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
  test('is reachable only in global settings', () => {
    expect(buildSettingsSubpages('permissions').some((page) => page.id === 'profiles')).toBe(true)
    expect(buildSettingsNavigationCategories().find((category) => category.id === 'security')?.children).toEqual(
      expect.arrayContaining([expect.objectContaining({ section: 'permissions', view: 'profiles' })]),
    )
    expect(buildSettingsScopes('permissions', 'profiles').map((scope) => scope.id)).toEqual(['global'])
  })

  test('creates and copies profiles, then persists group, user and conversation bindings with revision guards', async () => {
    const { panel, writes, requests, current } = await editor()
    await panel.addEntry()
    Object.assign(panel.draft.value, {
      id: 'readers',
      name: 'Readers',
      grantEnabled: true,
      toolsText: 'fs.read\nfs.read\nfs.list',
      mcpText: '{"docs":["search"]}',
      permissionText:
        '{"path":{"workspace":"readonly","external":"deny"},"tools":{"default":"ask","names":{"fs.read":"allow"}}}',
    })
    await panel.save()
    expect(panel.error.value).toBe('')
    expect(current().profiles.readers?.grant?.tools).toEqual(['fs.read', 'fs.list'])
    expect(current().profiles.readers?.revision).toBe(1)
    await panel.addEntry(true)
    panel.draft.value.id = 'researchers'
    panel.draft.value.name = 'Researchers'
    await panel.save()
    expect(current().profiles.researchers?.permission).toEqual(current().profiles.readers?.permission)

    await panel.selectSection('groups')
    await panel.addEntry()
    Object.assign(panel.draft.value, { id: 'staff', name: 'Staff', priority: 10, profile_id: 'readers' })
    await panel.save()
    await panel.selectSection('users')
    await panel.addEntry()
    Object.assign(panel.draft.value, {
      platform: 'telegram',
      tenant: 'telegram',
      account: 'bot-1',
      actor: '123',
      group_ids: ['staff'],
      profile_id: 'researchers',
      permissionText: '{"tools":{"names":{"shell":"deny"}}}',
    })
    await panel.save()
    await panel.selectSection('conversations')
    await panel.addEntry()
    Object.assign(panel.draft.value, {
      platform: 'telegram',
      tenant: 'telegram',
      account: 'bot-1',
      resource: '-100123',
      profile_id: 'readers',
    })
    await panel.save()
    expect(panel.error.value).toBe('')
    expect(current().users[0]).toMatchObject({
      actor: '123',
      group_ids: ['staff'],
      profile_id: 'researchers',
      overrides: { tools: { names: { shell: 'deny' } } },
    })
    expect(current().conversations[0]?.default_profile_id).toBe('readers')
    expect(writes.map((write) => write.expected_revision)).toEqual(['r1', 'r2', 'r3', 'r4', 'r5'])
    expect(requests.every((request) => request.url.startsWith('/api/v1/settings/layers/global'))).toBe(true)
    expect(panel.dirty.value).toBe(false)
  })

  test('requires removing profile and group bindings before deletion and supports reassignment', async () => {
    const catalog = permissionProfiles.emptyPermissionCatalog()
    catalog.profiles.reader = { name: 'Reader', description: '', revision: 1, permission: {} }
    catalog.groups.staff = { name: 'Staff', priority: 0, profile_id: 'reader' }
    catalog.users.push({
      platform: 'telegram',
      tenant: 'telegram',
      account: 'bot',
      actor: 'one',
      group_ids: ['staff'],
      overrides: {},
    })
    const { panel, writes, current } = await editor(catalog)
    await panel.removeEntry()
    expect(panel.error.value).toContain('in use')
    expect(writes).toHaveLength(0)
    await panel.selectSection('groups')
    await panel.removeEntry()
    expect(panel.error.value).toContain('in use')
    await panel.selectSection('users')
    panel.draft.value.actor = 'two'
    await panel.save()
    expect(current().users.map((user) => user.actor)).toEqual(['two'])
    await panel.removeEntry()
    await panel.selectSection('groups')
    await panel.removeEntry()
    await panel.selectSection('profiles')
    await panel.removeEntry()
    expect(current()).toEqual(permissionProfiles.emptyPermissionCatalog())
  })

  test('retains unsaved edits on server rejection and rejects an intervening write atomically', async () => {
    const catalog = permissionProfiles.emptyPermissionCatalog()
    catalog.profiles.reader = { name: 'Reader', description: '', revision: 1, permission: {} }
    const harness = await editor(catalog)
    harness.panel.draft.value.name = 'Changed'
    harness.fail('Conflicting groups at priority 10')
    await harness.panel.save()
    expect(harness.panel.error.value).toContain('Conflicting groups')
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

  test('reads omitted optional fields with the same defaults as the persisted schema', async () => {
    const catalog = permissionProfiles.readPermissionCatalog({
      profiles: { reader: { name: 'Reader' } },
      groups: { staff: { name: 'Staff', profile_id: 'reader' } },
      users: [{ platform: 'telegram', tenant: 'telegram', account: 'bot', actor: 'one' }],
    })
    const { panel, current } = await editor(catalog)
    expect(panel.draft.value.permissionText).toBe('{}')
    await panel.selectSection('groups')
    expect(panel.draft.value.priority).toBe(0)
    await panel.selectSection('users')
    expect(panel.draft.value.group_ids).toEqual([])
    panel.draft.value.group_ids = ['staff']
    await panel.save()
    expect(panel.error.value).toBe('')
    expect(current().users[0]?.group_ids).toEqual(['staff'])
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
