import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'

import * as permissionConfig from '../../src/lib/permissionConfig'

// Exercise the component's actual save/draft handlers with Vue reactivity and
// isolated transport dependencies. No live settings service or browser is used.
const source = readFileSync(new URL('../../src/components/settings/PermissionStudioPanel.vue', import.meta.url), 'utf8')
const { descriptor } = parse(source)
const compiled = transformSync(compileScript(descriptor, { id: 'permission-studio-test' }).content, {
  loader: 'ts',
  format: 'cjs',
}).code
const runtimeSettingsCompiled = transformSync(
  readFileSync(new URL('../../src/lib/runtimeSettings.ts', import.meta.url), 'utf8'),
  { loader: 'ts', format: 'cjs' },
).code
const scopes: vue.EffectScope[] = []
type PermissionSource = 'global' | 'workspace' | 'session' | 'effective'
export function disposePermissionEditors() {
  for (const scope of scopes.splice(0)) scope.stop()
}

export async function createPermissionEditor(
  initial: permissionConfig.PermissionConfig = {},
  inherited: permissionConfig.PermissionConfig | null = {
    path: { workspace: 'readwrite', external: 'deny' },
    network: { default: 'allow', inbound: 'deny', hostLoopback: 'deny' },
  },
  options: { section?: string; scope?: PermissionSource; useRuntimeTransport?: boolean } = {},
) {
  const writes: Array<{ path: string; value: unknown; scope: string }> = []
  const requests: Array<{ url: string; method: string; body: unknown }> = []
  const reads: string[] = []
  const serverPolicies: Record<PermissionSource, permissionConfig.PermissionConfig> = {
    global: {},
    workspace: {},
    session: {},
    effective: {},
  }
  serverPolicies[options.scope ?? 'global'] = structuredClone(initial)
  let parentPolicy = structuredClone(inherited)
  let failure = ''
  let saveResponse: Record<string, unknown> = {}
  let saveGate: Promise<void> | undefined
  const lastSessionId = vue.ref('1')
  const directory = vue.reactive({ currentDirectory: '/workspace/example' })
  const props = vue.reactive({
    section: options.section ?? 'network',
    scope: options.scope ?? 'global',
    sessionId: undefined as string | undefined,
  })
  const messages: Array<[string, string]> = []
  const translate = (key: string, values: Record<string, unknown> = {}) =>
    key.replace(/\{(\w+)\}/g, (_, name: string) => String(values[name] ?? '{' + name + '}'))
  async function persist(path: string, value: unknown, source: PermissionSource) {
    if (saveGate) await saveGate
    if (failure) throw new Error(failure)
    writes.push({ path, value: structuredClone(value), scope: source })
    const current = saveResponse.current !== undefined ? saveResponse.current : value
    serverPolicies[source] = structuredClone(current ?? {}) as permissionConfig.PermissionConfig
    return { changed: true, current: value ?? null, validated: true, ...structuredClone(saveResponse) }
  }
  const transport = {
    async setRuntimeSetting(path: string, value: unknown, _options: unknown, source: PermissionSource) {
      return await persist(path, value, source)
    },
    async deleteRuntimeSetting(path: string, _options: unknown, source: PermissionSource) {
      return await persist(path, undefined, source)
    },
  }
  const modules: Record<string, unknown> = {
    vue: { ...vue, onMounted() {}, onBeforeUnmount() {} },
    'vue-router': { onBeforeRouteLeave() {}, onBeforeRouteUpdate() {} },
    '@remixicon/vue': {},
    '@/components/settings/settingsScopeCatalog': { findSettingsScope: () => ({ label: 'Permissions' }) },
    '@/lib/api': {
      apiJson: async (url: string, init?: RequestInit) => {
        if (init?.method === 'PUT' || init?.method === 'DELETE') {
          const requestUrl = new URL(url, 'https://agena.test')
          const body = init.body ? JSON.parse(String(init.body)) : undefined
          requests.push({ url, method: init.method, body })
          const value = init.method === 'DELETE' ? undefined : body.value
          return await persist(
            body?.path ?? requestUrl.searchParams.get('path'),
            value,
            requestUrl.pathname.split('/').at(-1)! as PermissionSource,
          )
        }
        reads.push(url)
        if (url === '/api/v1/plugins/surface') return { permission_tools: [] }
        if (url.includes('/sessions/'))
          return {
            execution: {
              selected_permission: structuredClone(serverPolicies.session),
              inherited_permission: structuredClone(parentPolicy),
              effective_permission: structuredClone(serverPolicies.effective),
            },
          }
        const source = new URL(url, 'https://agena.test').pathname.split('/').at(-1)
        const policy = source === 'global' || source === 'workspace' ? serverPolicies[source] : serverPolicies.effective
        return { value: structuredClone(policy), inherited_value: structuredClone(parentPolicy) }
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
          serverPolicies.session = structuredClone(value) as permissionConfig.PermissionConfig
          return { execution: { selected_permission: structuredClone(serverPolicies.session) } }
        },
      }),
    },
    '@/lib/lastSession': { lastSessionId },
    '@/stores/directory': { useDirectoryStore: () => directory },
    '@/stores/toasts': {
      useToastsStore: () => ({ push: (kind: string, message: string) => messages.push([kind, message]) }),
    },
    '@/i18n/settingsText': { settingsText: translate },
    '@/lib/permissionConfig': permissionConfig,
  }
  if (options.useRuntimeTransport) {
    const runtimeModule = { exports: {} }
    const runtimeModules: Record<string, unknown> = {
      '@/lib/api': modules['@/lib/api'],
      './modelConfigurationApi': { notifyModelConfigurationChanged() {} },
      './resourceSync': { captureResourceObservation: () => ({ scope: 1 }) },
      './runtimeSettingReads': { retireRuntimeSettingReads() {}, readBatchedRuntimeSettingSources() {} },
    }
    new Function('require', 'module', 'exports', runtimeSettingsCompiled)(
      (id: string) => {
        if (!(id in runtimeModules)) throw new Error('Unexpected settings transport dependency: ' + id)
        return runtimeModules[id]
      },
      runtimeModule,
      runtimeModule.exports,
    )
    modules['@/lib/runtimeSettings'] = runtimeModule.exports
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
    requests,
    reads,
    messages,
    lastSessionId,
    directory,
    rejectSaves(message: string) {
      failure = message
    },
    setServerPolicy(value: permissionConfig.PermissionConfig, source = props.scope) {
      serverPolicies[source] = structuredClone(value)
    },
    setSaveResponse(value: Record<string, unknown>) {
      saveResponse = structuredClone(value)
    },
    holdSaves() {
      let release!: () => void
      saveGate = new Promise<void>((resolve) => {
        release = resolve
      })
      return () => {
        release()
        saveGate = undefined
      }
    },
    setInheritedPolicy(value: permissionConfig.PermissionConfig | null) {
      parentPolicy = structuredClone(value)
    },
    async switchScope(value: PermissionSource) {
      props.scope = value
      await vue.nextTick()
      await Promise.resolve()
      await vue.nextTick()
    },
  }
}
