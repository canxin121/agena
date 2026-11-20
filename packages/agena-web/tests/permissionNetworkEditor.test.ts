import { afterEach, describe, expect, test } from 'bun:test'
import * as vue from 'vue'
import { type NetworkPermissionConfig } from '../src/lib/permissionConfig'
import { createPermissionEditor as editor, disposePermissionEditors } from './helpers/permissionStudioHarness'

afterEach(disposePermissionEditors)

describe('flat network permission editor', () => {
  const networkFields = ['default', 'inbound', 'hostLoopback', 'allow', 'deny'] as const
  const offline: NetworkPermissionConfig = {
    version: 2,
    default: 'deny',
    inbound: 'deny',
    hostLoopback: 'deny',
    allow: [],
    deny: [],
  }
  const builtIn: NetworkPermissionConfig = { ...offline, default: 'allow' }

  test('directly editing global outbound access writes only that value into the JSON draft', async () => {
    const { panel, writes } = await editor()
    expect(panel.networkMode('default')).toBe('allow')
    expect(panel.networkMode('inbound')).toBe('deny')
    expect(panel.networkMode('hostLoopback')).toBe('deny')
    panel.setNetworkMode('default', 'allow')
    expect(JSON.parse(panel.networkJson.value)).toEqual({ version: 2, default: 'allow' })
    expect(panel.networkSource('inbound')).toBe('inherit')
    expect(panel.networkSource('hostLoopback')).toBe('inherit')
    expect(panel.config.value.path).toBeUndefined()
    expect(writes).toHaveLength(0)
    panel.setNetworkSource('default', 'inherit')
    expect(panel.config.value).toEqual({})
    expect(panel.networkMode('default')).toBe('allow')
  })

  test.each([...networkFields])('%s can be fixed at its inherited value and reset independently', async (field) => {
    const parent = {
      network: {
        ...builtIn,
        default: 'deny' as const,
        allow: [{ to: '192.0.2.10', protocol: 'tcp' as const, ports: [443] }],
        deny: [{ to: '192.0.2.11' }],
      },
    }
    const { panel, writes } = await editor({ tools: { default: 'ask' } }, parent, { scope: 'workspace' })
    expect(panel.networkCustom.value).toBe(false)
    expect(panel.displayedNetwork.value).toEqual(parent.network)
    panel.setNetworkSource(field, 'local')
    const local = { version: 2, [field]: parent.network[field] }
    expect(panel.config.value.network).toEqual(local)
    expect(panel.networkSource(field)).toBe('local')
    for (const other of networkFields.filter((key) => key !== field)) expect(panel.networkSource(other)).toBe('inherit')
    await panel.save()
    expect(writes[0]).toEqual({
      path: 'permission',
      value: { tools: { default: 'ask' }, network: local },
      scope: 'workspace',
    })
    panel.setNetworkSource(field, 'inherit')
    await panel.save()
    expect(writes[1]?.value).toEqual({ tools: { default: 'ask' } })
    expect(panel.networkSource(field)).toBe('inherit')
    expect(panel.displayedNetwork.value).toEqual(parent.network)
  })

  test.each([
    '{"allow":',
    JSON.stringify({ version: null }),
    JSON.stringify({ version: 3 }),
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
    const candidate = { version: 2, default: 'deny', allow: [{ to: '192.0.2.10', protocol: 'tcp', ports: [443] }] }
    panel.networkJson.value = JSON.stringify(candidate)
    rejectSaves('MXC Seatbelt does not support outbound rules')
    await panel.save()
    expect(writes).toHaveLength(0)
    expect(panel.saveError.value).toContain('Seatbelt')
    expect(JSON.parse(panel.networkJson.value)).toEqual(candidate)
    expect(panel.savedConfig.value.network).toEqual(offline)
    expect(panel.dirty.value).toBe(true)
  })

  test('legacy empty JSON is persisted as an explicit offline policy', async () => {
    const { panel, writes } = await editor({ network: { default: 'allow' } })
    panel.networkJson.value = '{}'
    await panel.save()
    expect(writes).toEqual([{ path: 'permission', value: { network: offline }, scope: 'global' }])
    expect(panel.networkCustom.value).toBe(true)
    expect(panel.networkMode('default')).toBe('deny')
    expect(panel.networkMode('inbound')).toBe('deny')
    expect(panel.networkMode('hostLoopback')).toBe('deny')
    expect(panel.dirty.value).toBe(false)
  })

  test('loading legacy empty network keeps all five offline settings local', async () => {
    const { panel, writes } = await editor({ network: {} }, { network: builtIn }, { scope: 'workspace' })
    expect(panel.config.value.network).toEqual(offline)
    expect(panel.networkMode('default')).toBe('deny')
    for (const field of networkFields) expect(panel.networkSource(field)).toBe('local')
    expect(writes).toHaveLength(0)
    panel.setNetworkMode('default', 'allow')
    await panel.save()
    expect(writes[0]?.value).toEqual({ network: builtIn })
  })

  test('a version-only policy inherits every field', async () => {
    const { panel, writes } = await editor({ network: { version: 2 } }, { network: builtIn })
    expect(panel.displayedNetwork.value).toEqual(builtIn)
    for (const field of networkFields) expect(panel.networkSource(field)).toBe('inherit')
    expect(panel.dirty.value).toBe(false)
    expect(writes).toHaveLength(0)
  })

  test('a missing inherited policy cannot be silently customized as offline', async () => {
    const { panel, writes } = await editor({}, null)
    for (const field of networkFields) {
      expect(panel.networkCanCustomize(field)).toBe(false)
      panel.setNetworkSource(field, 'local')
    }
    expect(panel.networkMode('default')).toBeUndefined()
    await panel.save()
    expect(panel.config.value.network).toBeUndefined()
    expect(panel.inheritedNetworkError.value).toContain('could not be loaded')
    expect(writes).toHaveLength(0)
  })

  test('an explicit scalar choice is writable when the parent is unknown', async () => {
    const { panel, writes } = await editor({}, null)
    panel.setNetworkMode('default', 'deny')
    await panel.save()
    expect(writes[0]?.value).toEqual({ network: { version: 2, default: 'deny' } })
    expect(panel.networkMode('inbound')).toBeUndefined()
    expect(panel.networkCanCustomize('allow')).toBe(false)
  })

  test('parent changes update every unmodified field while preserving a local scalar', async () => {
    const parent = { network: builtIn }
    const { panel, setInheritedPolicy } = await editor({}, parent, { scope: 'workspace' })
    panel.setNetworkSource('inbound', 'local')
    await panel.save()
    const changed: NetworkPermissionConfig = {
      version: 2,
      default: 'deny',
      inbound: 'allow',
      hostLoopback: 'allow',
      allow: [{ to: '192.0.2.10' }],
      deny: [{ to: '192.0.2.11' }],
    }
    setInheritedPolicy({ network: changed })
    await panel.refresh()
    expect(panel.config.value.network).toEqual({ version: 2, inbound: 'deny' })
    expect(panel.displayedNetwork.value).toEqual({ ...changed, inbound: 'deny' })
    expect(panel.networkBaseValueLabel('inbound')).toBe('Allow')
    panel.setNetworkSource('inbound', 'inherit')
    await panel.save()
    expect(panel.config.value.network).toBeUndefined()
    expect(panel.displayedNetwork.value).toEqual(changed)
  })

  test('an explicitly local allow list can be edited and cancelled without changing the deny source', async () => {
    const parent = {
      network: {
        ...builtIn,
        allow: [{ to: '192.0.2.10', protocol: 'tcp' as const, ports: [443] }],
        deny: [{ to: '192.0.2.11' }],
      },
    }
    const { panel, writes, setInheritedPolicy } = await editor({}, parent, { scope: 'workspace' })
    panel.editNetworkRule('allow', 0)
    expect(panel.config.value.network).toBeUndefined()
    expect(panel.networkRuleDraft.value).toBeNull()
    panel.setNetworkSource('allow', 'local')
    panel.editNetworkRule('allow', 0)
    panel.networkRuleDraft.value.ports = '8443'
    panel.cancelNetworkRule()
    expect(panel.config.value).toEqual({ network: { version: 2, allow: parent.network.allow } })
    expect(panel.dirty.value).toBe(true)
    panel.editNetworkRule('allow', 0)
    panel.networkRuleDraft.value.ports = '8443'
    expect(panel.applyNetworkRule()).toBe(true)
    const local = { version: 2, allow: [{ to: '192.0.2.10', protocol: 'tcp', ports: [8443] }] }
    expect(panel.config.value.network).toEqual(local)
    expect(panel.networkSource('deny')).toBe('inherit')
    expect(parent.network.allow[0]?.ports).toEqual([443])
    await panel.save()
    expect(writes[0]?.value).toEqual({ network: local })
    setInheritedPolicy({ network: { ...builtIn, allow: [{ to: '198.51.100.10' }], deny: [{ to: '198.51.100.11' }] } })
    await panel.refresh()
    expect(panel.networkRules('allow')).toEqual(local.allow)
    expect(panel.networkRules('deny')).toEqual([{ to: '198.51.100.11' }])
  })

  test('editing an explicitly local deny list leaves the allow list inherited', async () => {
    const parent = {
      network: {
        ...builtIn,
        allow: [{ to: '192.0.2.10' }],
        deny: [{ to: '192.0.2.11' }],
      },
    }
    const { panel } = await editor({}, parent, { scope: 'workspace' })
    panel.setNetworkSource('deny', 'local')
    panel.editNetworkRule('deny', 0)
    panel.networkRuleDraft.value.to = '192.0.2.12'
    expect(panel.applyNetworkRule()).toBe(true)
    expect(panel.config.value.network).toEqual({ version: 2, deny: [{ to: '192.0.2.12' }] })
    expect(panel.networkRules('allow')).toEqual(parent.network.allow)
    expect(panel.networkSource('allow')).toBe('inherit')
  })

  test.each(['allow', 'deny'] as const)(
    'deleting the last explicitly local %s rule fixes an empty list',
    async (action) => {
      const parent = {
        network: { ...builtIn, allow: [{ to: '192.0.2.10' }], deny: [{ to: '192.0.2.11' }] },
      }
      const { panel, writes } = await editor({}, parent, { scope: 'workspace' })
      const other = action === 'allow' ? 'deny' : 'allow'
      panel.removeNetworkRule(action, 0)
      expect(panel.config.value).toEqual({})
      panel.setNetworkSource(action, 'local')
      panel.removeNetworkRule(action, 0)
      expect(panel.config.value.network).toEqual({ version: 2, [action]: [] })
      expect(panel.networkSource(action)).toBe('local')
      expect(panel.networkSource(other)).toBe('inherit')
      expect(panel.networkRules(other)).toEqual(parent.network[other])
      await panel.save()
      expect(writes[0]?.value).toEqual({ network: { version: 2, [action]: [] } })
      panel.setNetworkSource(action, 'inherit')
      expect(panel.config.value.network).toBeUndefined()
      expect(panel.networkRules(action)).toEqual(parent.network[action])
    },
  )

  test('a complete network JSON draft replaces network settings only on save', async () => {
    const { panel, writes } = await editor({
      network: {
        default: 'allow',
        allow: [{ to: '*' }],
        deny: [{ to: '192.0.2.10' }],
        inbound: 'allow',
        hostLoopback: 'allow',
      },
    })
    panel.networkJson.value = JSON.stringify(offline)
    expect(panel.applyNetworkJson()).toBe(true)
    expect(panel.config.value.network).toEqual(offline)
    expect(writes).toHaveLength(0)
    await panel.save()
    expect(writes[0]?.value).toEqual({ network: offline })
    panel.networkJson.value = JSON.stringify(builtIn)
    expect(panel.applyNetworkJson()).toBe(true)
    expect(panel.config.value.network).toEqual(builtIn)
  })

  test('rule form accepts a literal target, comma-separated ports, ranges, and exclusions', async () => {
    const { panel, writes } = await editor({ network: { version: 2 } })
    panel.editNetworkRule('allow')
    Object.assign(panel.networkRuleDraft.value, {
      to: '10.20.0.0/16',
      ports: '443, 8000-8099',
      except: '10.20.10.0/24, 10.20.11.0/24',
    })
    await panel.save()
    expect(writes[0]?.value).toEqual({
      network: {
        version: 2,
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
    expect(panel.config.value.network).toEqual({ version: 2, allow: [] })
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

  test('editing the outbound default preserves existing rules', async () => {
    const { panel } = await editor({ network: { version: 2, default: 'allow', deny: [{ to: '192.0.2.10' }] } })
    panel.setNetworkMode('default', 'deny')
    expect(panel.config.value.network).toEqual({ version: 2, default: 'deny', deny: [{ to: '192.0.2.10' }] })
  })

  test('session customization uses a sparse v2 policy and the session save API', async () => {
    const parent = { network: { default: 'deny' as const, deny: [{ to: '192.0.2.10' }] } }
    const { panel, writes, sessionWrites, switchScope } = await editor({}, parent)
    await switchScope('session')
    panel.setNetworkSource('default', 'local')
    expect(panel.config.value.network).toEqual({ version: 2, default: 'deny' })
    panel.setNetworkMode('default', 'allow')
    await panel.save()
    expect(writes[0]?.scope).toBe('session')
    expect(sessionWrites).toEqual([{ sessionId: '1', value: { network: { version: 2, default: 'allow' } } }])
    expect(panel.config.value.network.deny).toBeUndefined()
    expect(panel.networkSource('deny')).toBe('inherit')
    expect(panel.displayedNetwork.value.deny).toEqual(parent.network.deny)
  })

  test('an in-flight network save remains attached to its original scope', async () => {
    const { panel, writes, holdSaves, switchScope } = await editor({}, { network: builtIn }, { scope: 'workspace' })
    panel.setNetworkSource('default', 'local')
    panel.setNetworkMode('default', 'deny')
    const release = holdSaves()
    const save = panel.save()
    await vue.nextTick()
    await switchScope('global')
    release()
    await save
    expect(writes).toEqual([
      { path: 'permission', value: { network: { version: 2, default: 'deny' } }, scope: 'workspace' },
    ])
    expect(panel.config.value).toEqual({})
    expect(panel.networkSource('default')).toBe('inherit')
    expect(panel.networkMode('default')).toBe('allow')
  })

  test('global network rules omit source labels while lower layers keep inheritance', async () => {
    const parent = { network: { ...builtIn, allow: [{ to: '192.0.2.10' }] } }
    const { panel, switchScope } = await editor({}, parent)
    expect(panel.baseSourceLabel.value).toBe('Default')
    expect(panel.restoreSourceLabel.value).toBe('Reset to defaults')
    expect(panel.ruleRowsByKind.value.network[0].sourceLabel).toBe('')
    await switchScope('workspace')
    expect(panel.baseSourceLabel.value).toBe('Inherit')
    expect(panel.restoreSourceLabel.value).toBe('Use inherited value')
    expect(panel.ruleRowsByKind.value.network[0].sourceLabel).toBe('Global layer')
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
