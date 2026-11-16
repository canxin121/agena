import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { parseNetworkPermissionConfig, type NetworkPermissionConfig } from '../src/lib/permissionConfig'
import { createPermissionEditor, disposePermissionEditors } from './helpers/permissionStudioHarness'

afterEach(disposePermissionEditors)

const examples = JSON.parse(
  readFileSync(
    new URL('../../../crates/agena-runtime-config/tests/fixtures/network_rule_examples.json', import.meta.url),
    'utf8',
  ),
) as Array<{ name: string; network: NetworkPermissionConfig }>

describe('shared native network examples in the Web editor', () => {
  test.each(examples)('$name saves in either layer without changing its selectors', async ({ network }) => {
    expect(parseNetworkPermissionConfig(network)).toEqual(network)
    for (const scope of ['global', 'workspace']) {
      const unrelated = { tools: { default: 'ask' as const } }
      const { panel, requests } = await createPermissionEditor(
        unrelated,
        { network: { default: 'allow', deny: [{ to: '198.51.100.10' }] } },
        { section: 'network', scope, useRuntimeTransport: true },
      )
      panel.setNetworkSource('custom')
      panel.networkJson.value = JSON.stringify(network)
      await panel.save()
      expect(requests).toEqual([
        {
          url: `/api/v1/settings/layers/${scope}`,
          method: 'PUT',
          body: { path: 'permission', value: { ...unrelated, network }, dry_run: false, validate: true, reload: true },
        },
      ])
      expect(panel.savedConfig.value.network).toEqual(network)
      expect(panel.networkCustom.value).toBe(true)
      expect(panel.dirty.value).toBe(false)
    }
  })

  test('replacing inherited network does not retain its denies, and clearing custom mode restores inheritance', async () => {
    const parent = { network: { default: 'allow' as const, deny: [{ to: '192.0.2.99' }] } }
    const { panel, requests } = await createPermissionEditor({}, parent, {
      section: 'network',
      scope: 'workspace',
      useRuntimeTransport: true,
    })
    panel.setNetworkSource('custom')
    const replacement = { default: 'deny', allow: [{ to: '192.0.2.99', protocol: 'tcp', ports: [443] }] }
    panel.networkJson.value = JSON.stringify(replacement)
    await panel.save()
    expect(panel.savedConfig.value.network).toEqual(replacement)
    expect(panel.displayedNetwork.value).toEqual(replacement)
    panel.setNetworkSource('inherit')
    await panel.save()
    expect(panel.savedConfig.value).toEqual({})
    expect(panel.networkCustom.value).toBe(false)
    expect(panel.displayedNetwork.value).toEqual(parent.network)
    expect(requests[1]).toEqual({
      url: '/api/v1/settings/layers/workspace?path=permission&dry_run=false&validate=true&reload=true',
      method: 'DELETE',
      body: undefined,
    })
  })

  test.each([
    {
      network: { allow: [{ to: '192.0.2.10', protocol: 'tcp', ports: [443] }] },
      error: 'permission.network.allow: macOS Seatbelt does not support CIDR, protocol, or port rules',
    },
    { network: { inbound: 'allow' }, error: 'permission.network.inbound: Linux Bubblewrap does not support allow' },
    {
      network: { hostLoopback: 'allow' },
      error: 'permission.network.hostLoopback: Linux Bubblewrap does not support allow',
    },
  ])(
    'native capability errors remain authoritative and preserve the authored policy: $error',
    async ({ network, error }) => {
      const { panel, requests, writes, rejectSaves } = await createPermissionEditor({ network: {} }, null, {
        section: 'network',
        useRuntimeTransport: true,
      })
      panel.networkJson.value = JSON.stringify(network)
      rejectSaves(error)
      await panel.save()
      expect(requests).toHaveLength(1)
      expect(writes).toHaveLength(0)
      expect(panel.saveError.value).toBe(error)
      expect(panel.config.value.network).toEqual(network)
      expect(panel.savedConfig.value.network).toEqual({})
      expect(panel.dirty.value).toBe(true)
    },
  )
})
