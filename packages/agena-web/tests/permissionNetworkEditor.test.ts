import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'

import * as permissionConfig from '../src/lib/permissionConfig'

// Exercise the component's actual save/draft handlers with Vue reactivity and
// isolated transport dependencies. No live settings service or browser is used.
const source = readFileSync(new URL('../src/components/settings/PermissionStudioPanel.vue', import.meta.url), 'utf8')
const { descriptor } = parse(source)
const compiled = transformSync(compileScript(descriptor, { id: 'permission-network-test' }).content, {
  loader: 'ts',
  format: 'cjs',
}).code
const scopes: vue.EffectScope[] = []
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
})

async function editor(initial: permissionConfig.PermissionConfig = {}) {
  const writes: Array<{ path: string; value: unknown; scope: string }> = []
  let serverPolicy = structuredClone(initial)
  let failure = ''
  const props = vue.reactive({ section: 'network', scope: 'global', sessionId: undefined })
  const messages: Array<[string, string]> = []
  const translate = (key: string, values: Record<string, unknown> = {}) =>
    key.replace(/\{(\w+)\}/g, (_, name: string) => String(values[name] ?? '{' + name + '}'))
  const transport = {
    async setRuntimeSetting(path: string, value: unknown, _options: unknown, scope: string) {
      if (failure) throw new Error(failure)
      writes.push({ path, value: structuredClone(value), scope })
      serverPolicy = structuredClone(value) as permissionConfig.PermissionConfig
    },
    async deleteRuntimeSetting(path: string, _options: unknown, scope: string) {
      if (failure) throw new Error(failure)
      writes.push({ path, value: undefined, scope })
      serverPolicy = {}
    },
  }
  const modules: Record<string, unknown> = {
    vue: { ...vue, onMounted() {}, onBeforeUnmount() {} },
    'vue-router': { onBeforeRouteLeave() {}, onBeforeRouteUpdate() {} },
    '@remixicon/vue': {},
    '@/components/settings/settingsScopeCatalog': { findSettingsScope: () => ({ label: 'Permissions' }) },
    '@/lib/api': {
      apiJson: async (url: string) =>
        url === '/api/v1/plugins/surface' ? { permission_tools: [] } : { value: structuredClone(serverPolicy) },
    },
    '@/lib/appConfirm': { confirmAction: async () => true },
    '@/lib/runtimeSettings': transport,
    '@/stores/chat': { useChatStore: () => ({ selectedSessionId: '1' }) },
    '@/lib/lastSession': { lastSessionId: vue.ref('1') },
    '@/stores/directory': { useDirectoryStore: () => ({ currentDirectory: '/workspace/example' }) },
    '@/stores/toasts': {
      useToastsStore: () => ({ push: (kind: string, message: string) => messages.push([kind, message]) }),
    },
    '@/i18n/settingsText': { settingsText: translate },
    '@/lib/permissionConfig': permissionConfig,
  }
  const module = { exports: {} as { default?: { setup: (props: unknown, context: unknown) => any } } }
  const require = (id: string) => {
    if (id.endsWith('.vue')) return {}
    if (!(id in modules)) throw new Error('Unexpected component dependency: ' + id)
    return modules[id]
  }
  new Function('require', 'module', 'exports', compiled)(require, module, module.exports)
  const scope = vue.effectScope()
  scopes.push(scope)
  const panel = scope.run(() => module.exports.default!.setup(props, { expose() {} }))!
  await panel.load()
  return {
    panel,
    props,
    writes,
    messages,
    rejectSaves(message: string) {
      failure = message
    },
    setServerPolicy(value: permissionConfig.PermissionConfig) {
      serverPolicy = structuredClone(value)
    },
    async switchScope(value: string) {
      props.scope = value
      await vue.nextTick()
      await Promise.resolve()
      await vue.nextTick()
    },
  }
}

describe('native network permission editor', () => {
  test.each([
    '{"egress":',
    JSON.stringify({ internet: 'allow' }),
    JSON.stringify({ egress: { allow: [{ ports: [{ protocol: 'icmp', port: 443 }] }] } }),
  ])('invalid network draft never reaches persistence: %s', async (draft) => {
    const { panel, writes } = await editor({ network: { egress: { default: 'deny' } } })
    const original = structuredClone(vue.toRaw(panel.savedConfig.value))
    panel.networkJson.value = draft

    await panel.save()

    expect(writes).toHaveLength(0)
    expect(panel.networkJson.value).toBe(draft)
    expect(panel.networkJsonError.value).not.toBe('')
    expect(panel.savedConfig.value).toEqual(original)
    expect(panel.dirty.value).toBe(true)
  })

  test('backend capability rejection retains the editable draft and original saved policy', async () => {
    const { panel, writes, rejectSaves } = await editor({ network: { egress: { default: 'deny' } } })
    const candidate = {
      egress: { default: 'deny', allow: [{ to: [{ cidr: '192.0.2.10/32' }] }] },
    }
    panel.networkJson.value = JSON.stringify(candidate)
    rejectSaves('MXC Seatbelt does not support egress rules')

    await panel.save()

    expect(writes).toHaveLength(0)
    expect(panel.saveError.value).toContain('Seatbelt')
    expect(JSON.parse(panel.networkJson.value)).toEqual(candidate)
    expect(panel.savedConfig.value.network.egress.default).toBe('deny')
    expect(panel.savedConfig.value.network.egress.allow).toBeUndefined()
    expect(panel.dirty.value).toBe(true)
  })

  test('explicit empty lists are persisted as overrides and successful save clears dirty state', async () => {
    const { panel, writes } = await editor({
      network: { egress: { default: 'deny', allow: [{ to: [{ cidr: '192.0.2.10/32' }] }] } },
    })
    panel.networkJson.value = '{"egress":{"allow":[],"deny":[]}}'

    await panel.save()

    expect(writes).toEqual([
      { path: 'permission', value: { network: { egress: { allow: [], deny: [] } } }, scope: 'global' },
    ])
    expect(panel.dirty.value).toBe(false)
    expect(panel.networkJsonError.value).toBe('')
  })

  test('refresh and advanced JSON apply do not discard an unfinished network draft', async () => {
    const { panel, setServerPolicy } = await editor()
    panel.networkJson.value = '{"egress":'
    setServerPolicy({ network: { egress: { default: 'allow' } } })

    await panel.refresh()
    expect(panel.applyRawJson()).toBe(false)
    expect(panel.networkJson.value).toBe('{"egress":')
    expect(panel.dirty.value).toBe(true)
  })

  test('switching setting scopes restores invalid network draft', async () => {
    const { panel, switchScope } = await editor()
    panel.networkJson.value = '{"egress":'

    await switchScope('workspace')
    expect(panel.networkJson.value).toBe('{}')
    await switchScope('global')

    expect(panel.networkJson.value).toBe('{"egress":')
    expect(panel.dirty.value).toBe(true)
  })

  test('clearing a default inherits without removing explicit array overrides', async () => {
    const { panel, writes } = await editor({ network: { egress: { default: 'allow', allow: [] } } })
    panel.setNetworkMode('egress', '')
    await panel.save()

    expect(writes[0]?.value).toEqual({ network: { egress: { allow: [] } } })
  })
})
