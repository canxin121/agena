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

async function editor(
  initial: permissionConfig.PermissionConfig = {},
  inherited: permissionConfig.PermissionConfig | null = { network: { default: 'deny' } },
) {
  const writes: Array<{ path: string; value: unknown; scope: string }> = []
  let serverPolicy = structuredClone(initial)
  let parentPolicy = structuredClone(inherited)
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
      apiJson: async (url: string) => {
        if (url === '/api/v1/plugins/surface') return { permission_tools: [] }
        if (url.includes('/sessions/'))
          return {
            execution: {
              selected_permission: structuredClone(serverPolicy),
              inherited_permission: structuredClone(parentPolicy),
              effective_permission: structuredClone(serverPolicy),
            },
          }
        return { value: structuredClone(serverPolicy), inherited_value: structuredClone(parentPolicy) }
      },
    },
    '@/lib/appConfirm': { confirmAction: async () => true },
    '@/lib/runtimeSettings': transport,
    '@/stores/chat': {
      useChatStore: () => ({
        selectedSessionId: '1',
        async setSessionPermission(_id: string, value: unknown) {
          if (failure) throw new Error(failure)
          writes.push({ path: 'permission', value: structuredClone(value), scope: 'session' })
          serverPolicy = structuredClone(value) as permissionConfig.PermissionConfig
          return { execution: { selected_permission: structuredClone(serverPolicy) } }
        },
      }),
    },
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
    setInheritedPolicy(value: permissionConfig.PermissionConfig | null) {
      parentPolicy = structuredClone(value)
    },
    async switchScope(value: string) {
      props.scope = value
      await vue.nextTick()
      await Promise.resolve()
      await vue.nextTick()
    },
  }
}

describe('flat network permission editor', () => {
  test.each([
    '{"allow":',
    JSON.stringify({ internet: 'allow' }),
    JSON.stringify({ egress: { default: 'allow' } }),
    JSON.stringify({ allow: [{ to: '*', protocol: 'icmp', ports: [443] }] }),
    JSON.stringify({ allow: [{ to: null }] }),
    JSON.stringify({ allow: [{ to: '*', ports: [null] }] }),
    JSON.stringify({ allow: [{ to: '192.0.2.7/24' }] }),
  ])('invalid network JSON never reaches persistence: %s', async (draft) => {
    const { panel, writes } = await editor({ network: { default: 'deny' } })
    const original = structuredClone(vue.toRaw(panel.savedConfig.value))
    panel.networkJson.value = draft
    await panel.save()
    expect(writes).toHaveLength(0)
    expect(panel.networkJson.value).toBe(draft)
    expect(panel.networkJsonError.value).not.toBe('')
    expect(panel.savedConfig.value).toEqual(original)
    expect(panel.dirty.value).toBe(true)
  })

  test('backend capability rejection retains the editable policy and original saved value', async () => {
    const { panel, writes, rejectSaves } = await editor({ network: { default: 'deny' } })
    const candidate = { default: 'deny', allow: [{ to: '192.0.2.10', protocol: 'tcp', ports: [443] }] }
    panel.networkJson.value = JSON.stringify(candidate)
    rejectSaves('MXC Seatbelt does not support outbound rules')
    await panel.save()
    expect(writes).toHaveLength(0)
    expect(panel.saveError.value).toContain('Seatbelt')
    expect(JSON.parse(panel.networkJson.value)).toEqual(candidate)
    expect(panel.savedConfig.value.network).toEqual({ default: 'deny' })
    expect(panel.dirty.value).toBe(true)
  })

  test('empty JSON is persisted as an explicit policy, not inheritance', async () => {
    const { panel, writes } = await editor({ network: { default: 'allow' } })
    panel.networkJson.value = '{}'
    await panel.save()
    expect(writes).toEqual([{ path: 'permission', value: { network: {} }, scope: 'global' }])
    expect(panel.networkCustom.value).toBe(true)
    expect(panel.dirty.value).toBe(false)
  })

  test('customizing copies the complete parent policy and returning to inherit removes only network', async () => {
    const parent = {
      network: {
        default: 'deny' as const,
        allow: [{ to: '192.0.2.10', protocol: 'tcp' as const, ports: [443] }],
        deny: [{ to: '192.0.2.11' }],
      },
    }
    const { panel, writes } = await editor({ tools: { default: 'ask' } }, parent)
    expect(panel.networkCustom.value).toBe(false)
    expect(panel.displayedNetwork.value).toEqual(parent.network)
    panel.setNetworkSource('custom')
    expect(panel.config.value.network).toEqual(parent.network)
    panel.setNetworkMode('default', 'allow')
    expect(panel.config.value.network.deny).toEqual(parent.network.deny)
    expect(parent.network.default).toBe('deny')
    await panel.save()
    expect((writes[0]?.value as permissionConfig.PermissionConfig).network?.deny).toEqual(parent.network.deny)
    panel.setNetworkSource('inherit')
    await panel.save()
    expect(writes[1]?.value).toEqual({ tools: { default: 'ask' } })
    expect(panel.displayedNetwork.value).toEqual(parent.network)
  })

  test('a missing inherited policy cannot be silently customized as offline', async () => {
    const { panel, writes } = await editor({}, null)
    panel.setNetworkSource('custom')
    await panel.save()
    expect(panel.config.value.network).toBeUndefined()
    expect(panel.inheritedNetworkError.value).toContain('could not be loaded')
    expect(writes).toHaveLength(0)
  })

  test('presets replace the complete network draft without saving automatically', async () => {
    const { panel, writes } = await editor({
      network: {
        default: 'allow',
        allow: [{ to: '*' }],
        deny: [{ to: '192.0.2.10' }],
        inbound: 'allow',
        hostLoopback: 'allow',
      },
    })
    panel.useNetworkPreset('deny')
    expect(panel.config.value.network).toEqual({ default: 'deny' })
    expect(writes).toHaveLength(0)
    await panel.save()
    expect(writes[0]?.value).toEqual({ network: { default: 'deny' } })
    panel.useNetworkPreset('allow')
    expect(panel.config.value.network).toEqual({ default: 'allow' })
  })

  test('rule form accepts a literal target, comma-separated ports, ranges, and exclusions', async () => {
    const { panel, writes } = await editor({ network: {} })
    panel.editNetworkRule('allow')
    Object.assign(panel.networkRuleDraft.value, {
      to: '10.20.0.0/16',
      ports: '443, 8000-8099',
      except: '10.20.10.0/24, 10.20.11.0/24',
    })
    await panel.save()
    expect(writes[0]?.value).toEqual({
      network: {
        allow: [
          {
            to: '10.20.0.0/16',
            protocol: 'tcp',
            ports: [443, '8000-8099'],
            except: ['10.20.10.0/24', '10.20.11.0/24'],
          },
        ],
      },
    })
    expect(panel.networkRuleDraft.value).toBeNull()
    expect(panel.dirty.value).toBe(false)
    panel.editNetworkRule('allow', 0)
    panel.networkRuleDraft.value.ports = '8443'
    expect(panel.applyNetworkRule()).toBe(true)
    expect(panel.config.value.network.allow[0].ports).toEqual([8443])
    panel.removeNetworkRule('allow', 0)
    expect(panel.config.value.network).toEqual({})
    expect(panel.networkCustom.value).toBe(true)
  })

  test('invalid rule input survives failed save, refresh, and scope switches', async () => {
    const { panel, writes, switchScope } = await editor({ network: { default: 'deny' } })
    panel.editNetworkRule('allow')
    Object.assign(panel.networkRuleDraft.value, { to: '192.0.2.10', ports: '443, nonsense' })
    await panel.save()
    expect(writes).toHaveLength(0)
    expect(panel.networkRuleError.value).toContain('ports[1]')
    expect(panel.networkRuleDraft.value.ports).toBe('443, nonsense')
    await panel.refresh()
    expect(panel.networkRuleDraft.value.ports).toBe('443, nonsense')
    await switchScope('workspace')
    expect(panel.networkRuleDraft.value).toBeNull()
    await switchScope('global')
    expect(panel.networkRuleDraft.value.ports).toBe('443, nonsense')
    panel.networkRuleDraft.value.ports = '443'
    await panel.save()
    expect(writes).toHaveLength(1)
    expect(panel.dirty.value).toBe(false)
  })

  test('switching a rule to ICMP clears its inapplicable ports', async () => {
    const { panel } = await editor({ network: { allow: [{ to: '192.0.2.10', protocol: 'tcp', ports: [443] }] } })
    panel.editNetworkRule('allow', 0)
    panel.setNetworkRuleProtocol('icmp')
    expect(panel.networkRuleDraft.value.ports).toBe('')
    expect(panel.applyNetworkRule()).toBe(true)
    expect(panel.config.value.network.allow).toEqual([{ to: '192.0.2.10', protocol: 'icmp' }])
  })

  test('default edits preserve existing rules while presets explicitly replace them', async () => {
    const { panel } = await editor({ network: { default: 'allow', deny: [{ to: '192.0.2.10' }] } })
    panel.setNetworkMode('default', 'deny')
    expect(panel.config.value.network).toEqual({ default: 'deny', deny: [{ to: '192.0.2.10' }] })
    panel.useNetworkPreset('deny')
    expect(panel.config.value.network).toEqual({ default: 'deny' })
  })

  test('session customization uses its inherited permission and session save API', async () => {
    const parent = { network: { default: 'deny' as const, deny: [{ to: '192.0.2.10' }] } }
    const { panel, writes, switchScope } = await editor({}, parent)
    await switchScope('session')
    panel.setNetworkSource('custom')
    expect(panel.config.value.network).toEqual(parent.network)
    panel.setNetworkMode('default', 'allow')
    await panel.save()
    expect(writes[0]?.scope).toBe('session')
    expect(panel.config.value.network.deny).toEqual(parent.network.deny)
  })

  test('refresh and advanced permission JSON do not discard an unfinished network JSON draft', async () => {
    const { panel, setServerPolicy } = await editor({ network: {} })
    panel.networkJson.value = '{"allow":'
    setServerPolicy({ network: { default: 'allow' } })
    await panel.refresh()
    expect(panel.applyRawJson()).toBe(false)
    expect(panel.networkJson.value).toBe('{"allow":')
    expect(panel.dirty.value).toBe(true)
  })

  test('resetting the scope cancels a pending rule instead of saving it back', async () => {
    const { panel, writes } = await editor({ network: { default: 'allow' } })
    panel.editNetworkRule('allow')
    panel.networkRuleDraft.value.to = '192.0.2.10'
    await panel.clearSource()
    expect(panel.networkRuleDraft.value).toBeNull()
    await panel.save()
    expect(writes).toEqual([{ path: 'permission', value: undefined, scope: 'global' }])
  })
})
