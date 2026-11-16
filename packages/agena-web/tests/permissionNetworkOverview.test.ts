import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'
import * as permissionConfig from '../src/lib/permissionConfig'

const source = readFileSync(new URL('../src/components/settings/PermissionsOverviewPanel.vue', import.meta.url), 'utf8')
const { descriptor } = parse(source)
const compiled = transformSync(compileScript(descriptor, { id: 'native-network-overview-test' }).content, {
  loader: 'ts',
  format: 'cjs',
}).code
const scopes: vue.EffectScope[] = []
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
})

async function overview(policy: permissionConfig.PermissionConfig, cardId = 'network') {
  const modules: Record<string, unknown> = {
    vue: { ...vue, onMounted() {}, onBeforeUnmount() {} },
    'vue-router': { useRoute: () => ({}), useRouter: () => ({ push() {} }) },
    '@remixicon/vue': {},
    '@/lib/api': { apiJson: async () => ({ value: policy }) },
    '@/stores/chat': { useChatStore: () => ({ selectedSessionId: undefined }) },
    '@/lib/lastSession': { lastSessionId: vue.ref(undefined) },
    '@/i18n/settingsText': { settingsText: (key: string) => key },
    '@/lib/permissionConfig': permissionConfig,
  }
  const module = { exports: {} as { default?: { setup: (props: unknown, context: unknown) => any } } }
  const require = (id: string) => {
    if (id.endsWith('.vue')) return {}
    if (!(id in modules)) throw new Error('Unexpected overview dependency: ' + id)
    return modules[id]
  }
  new Function('require', 'module', 'exports', compiled)(require, module, module.exports)
  const scope = vue.effectScope()
  scopes.push(scope)
  const panel = scope.run(() => module.exports.default!.setup({}, { expose() {} }))!
  await panel.refresh()
  expect(panel.error.value).toBe('')
  return panel.cards.value.find((card: { id: string }) => card.id === cardId)
}

describe('native network permissions overview', () => {
  test('shows the server-resolved built-in workspace and network defaults', async () => {
    const policy: permissionConfig.PermissionConfig = {
      path: { workspace: 'readwrite', external: 'deny' },
      network: { default: 'allow', inbound: 'deny', hostLoopback: 'deny' },
    }
    const files = await overview(policy, 'filesystem')
    expect(files.rows).toEqual([
      { label: 'Workspace files', mode: 'readwrite' },
      { label: 'External files', mode: 'deny' },
    ])
    const network = await overview(policy)
    expect(network.rows.map((row: { mode: string }) => row.mode)).toEqual(['allow', 'deny', 'deny'])
  })

  test('an absent effective policy is not presented as explicit offline access', async () => {
    const card = await overview({})
    expect(card.rows.map((row: { mode?: string }) => row.mode)).toEqual([undefined, undefined, undefined])
  })

  test('shows native effective actions and counts allow plus deny selectors', async () => {
    const card = await overview({
      network: {
        default: 'deny',
        allow: [{ to: '192.0.2.10', protocol: 'tcp', ports: [443] }],
        deny: [{ to: '192.0.2.11' }],
        inbound: 'allow',
        hostLoopback: 'deny',
      },
    })
    expect(card.count).toBe(2)
    expect(card.rows).toEqual([
      { label: 'Outbound default', mode: 'deny' },
      { label: 'Inbound default', mode: 'allow' },
      { label: 'Host loopback', mode: 'deny' },
    ])
  })

  test('an explicit empty network policy displays offline defaults', async () => {
    const card = await overview({ network: {} })
    expect(card.count).toBe(0)
    expect(card.rows.map((row: { mode: string }) => row.mode)).toEqual(['deny', 'deny', 'deny'])
  })
})
