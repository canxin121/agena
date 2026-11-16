import { afterEach, describe, expect, test } from 'bun:test'
import * as vue from 'vue'
import { createPermissionEditor as editor, disposePermissionEditors } from './helpers/permissionStudioHarness'

afterEach(disposePermissionEditors)

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
