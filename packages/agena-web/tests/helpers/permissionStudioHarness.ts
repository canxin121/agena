import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'

import * as permissionConfig from '../../src/lib/permissionConfig'
import * as permissionProfiles from '../../src/lib/permissionProfiles'
import { runtimeSettingBaseValue } from '../../src/lib/runtimeSettingLayer'
import * as permissionNavigation from '../../src/components/settings/permissions/permissionNavigation'
import { resolveSettingsSessionId } from '../../src/components/settings/settingsSessionScope'

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
  options: {
    section?: string
    scope?: PermissionSource
    useRuntimeTransport?: boolean
    configPaths?: Partial<Record<'global' | 'workspace', string>>
    sessionId?: string
    windowId?: string
    focused?: boolean
    resolvedPolicy?: permissionConfig.PermissionConfig
    legacySettingsMetadata?: boolean
    profileDraft?: boolean
    profiles?: permissionProfiles.PermissionCatalog['profiles']
    factoryProfiles?: permissionProfiles.PermissionCatalog['profiles']
  } = {},
) {
  const writes: Array<{ path: string; value: unknown; scope: string }> = []
  const requests: Array<{ url: string; method: string; body: unknown }> = []
  const reads: string[] = []
  const sessionWrites: Array<{ sessionId: string; value: unknown }> = []
  const sessionPolicies = new Map<string, permissionConfig.PermissionConfig>()
  const sessionProfiles = new Map<string, string>()
  const fileProfiles: Partial<Record<PermissionSource, string>> = {}
  const sessionReadGates = new Map<string, Promise<void>>()
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
  let confirmation = true
  const confirmations: Array<Record<string, unknown>> = []
  let routeUpdate: ((to: any, from: any) => Promise<boolean>) | undefined
  let routeLeave: ((to: any, from: any) => Promise<boolean> | boolean) | undefined
  const route = vue.reactive({
    path: '/settings/permissions',
    params: { section: 'permissions' },
    query: {
      view: options.section ?? 'access',
      scope: options.scope ?? 'global',
      ...(options.sessionId ? { targetSessionId: options.sessionId } : {}),
      ...(options.windowId ? { windowId: options.windowId } : {}),
    } as Record<string, string>,
    hash: '',
  })
  const lastSessionId = vue.ref('1')
  const workspacePane = options.windowId
    ? { windowId: vue.ref(options.windowId), isFocused: vue.ref(options.focused !== false) }
    : null
  const directory = vue.reactive({ currentDirectory: '/workspace/example' })
  const props = vue.reactive({
    scope: options.scope ?? 'global',
    sessionId: options.sessionId as string | undefined,
    draftPermission: options.profileDraft ? structuredClone(initial) : undefined,
    inheritedDraftPermission: options.profileDraft ? structuredClone(inherited) : undefined,
  })
  const messages: Array<[string, string]> = []
  const translate = (key: string, values: Record<string, unknown> = {}) =>
    key.replace(/\{(\w+)\}/g, (_, name: string) => String(values[name] ?? '{' + name + '}'))
  async function persist(path: string, value: unknown, source: PermissionSource) {
    if (saveGate) await saveGate
    if (failure) throw new Error(failure)
    writes.push({ path, value: structuredClone(value), scope: source })
    const current = saveResponse.current !== undefined ? saveResponse.current : value
    if (path === 'permission_profile') fileProfiles[source] = typeof current === 'string' ? current : undefined
    else serverPolicies[source] = structuredClone(current ?? {}) as permissionConfig.PermissionConfig
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
    vue: { ...vue, onMounted() {}, onBeforeUnmount() {}, useId: () => 'permission-studio-test' },
    'vue-router': {
      onBeforeRouteLeave(guard: typeof routeLeave) {
        routeLeave = guard
      },
      onBeforeRouteUpdate(guard: typeof routeUpdate) {
        routeUpdate = guard
      },
      useRoute: () => route,
      useRouter: () => ({
        replace: async (destination: any) => {
          Object.assign(route, destination)
        },
      }),
    },
    '@remixicon/vue': {},
    'radix-vue': {
      PopoverContent: { name: 'PopoverContent' },
      PopoverPortal: { name: 'PopoverPortal' },
      PopoverRoot: { name: 'PopoverRoot' },
      PopoverTrigger: { name: 'PopoverTrigger' },
    },
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
        if (url.includes('/sessions/')) {
          const sessionId = url.match(/\/sessions\/([^/]+)\//)?.[1] || '1'
          const selected = structuredClone(sessionPolicies.get(sessionId) ?? serverPolicies.session)
          const profile = sessionProfiles.get(sessionId)
          const effective = profile
            ? permissionConfig.completePermissionConfig(options.profiles?.[profile]?.permission ?? {})
            : structuredClone(sessionPolicies.get(sessionId) ?? serverPolicies.effective)
          await sessionReadGates.get(sessionId)
          return {
            execution: {
              selected_permission: selected,
              permission_profile: profile,
              inherited_permission: structuredClone(parentPolicy),
              effective_permission: effective,
            },
          }
        }
        const request = new URL(url, 'https://agena.test')
        const source = request.pathname.split('/').at(-1)
        const path = request.searchParams.get('path')
        if (path === 'access_control')
          return {
            value: { profiles: structuredClone(options.profiles ?? {}) },
            default_value: { profiles: structuredClone(options.factoryProfiles ?? {}) },
          }
        const profile = source === 'global' || source === 'workspace' ? fileProfiles[source] : undefined
        const implicitDefault =
          source === 'global' &&
          profile === undefined &&
          !Object.keys(serverPolicies.global).length &&
          options.factoryProfiles?.[permissionProfiles.DEFAULT_PERMISSION_PROFILE_ID]
            ? permissionProfiles.DEFAULT_PERMISSION_PROFILE_ID
            : undefined
        const activeProfile = profile ?? implicitDefault
        if (path === 'permission_profile') return { value: profile ?? null, layer_value: activeProfile ?? null }
        const policy = source === 'global' || source === 'workspace' ? serverPolicies[source] : serverPolicies.effective
        return {
          value: structuredClone(policy),
          ...(source === 'global' && !options.legacySettingsMetadata
            ? { default_value: structuredClone(parentPolicy) }
            : { inherited_value: structuredClone(parentPolicy) }),
          ...(activeProfile
            ? {
                layer_value: permissionConfig.completePermissionConfig(
                  options.profiles?.[activeProfile]?.permission ??
                    options.factoryProfiles?.[activeProfile]?.permission ??
                    {},
                ),
              }
            : options.resolvedPolicy !== undefined
              ? { layer_value: structuredClone(options.resolvedPolicy) }
              : {}),
          config_path: source === 'global' || source === 'workspace' ? options.configPaths?.[source] : undefined,
        }
      },
    },
    '@/lib/appConfirm': {
      confirmAction: async (options: Record<string, unknown>) => {
        confirmations.push(options)
        return confirmation
      },
    },
    '@/lib/runtimeSettings': transport,
    '@/stores/chat': {
      useChatStoreUnscoped: () => ({
        selectedSessionId: '1',
        async setSessionPermission(id: string, value: unknown) {
          if (saveGate) await saveGate
          if (failure) throw new Error(failure)
          sessionWrites.push({ sessionId: id, value: structuredClone(value) })
          writes.push({ path: 'permission', value: structuredClone(value), scope: 'session' })
          serverPolicies.session = structuredClone(value) as permissionConfig.PermissionConfig
          sessionPolicies.set(id, structuredClone(value) as permissionConfig.PermissionConfig)
          sessionProfiles.delete(id)
          return { execution: { selected_permission: structuredClone(serverPolicies.session) } }
        },
        async setSessionPermissionProfile(id: string, profile: string | null) {
          if (saveGate) await saveGate
          if (failure) throw new Error(failure)
          if (profile) sessionProfiles.set(id, profile)
          else sessionProfiles.delete(id)
          writes.push({ path: 'permission_profile', value: profile, scope: 'session' })
        },
      }),
    },
    '@/lib/lastSession': { lastSessionId },
    '@/app/workspace/workspacePaneContext': { useWorkspacePaneContext: () => workspacePane },
    '@/stores/directory': { useDirectoryStore: () => directory },
    '@/stores/toasts': {
      useToastsStore: () => ({ push: (kind: string, message: string) => messages.push([kind, message]) }),
    },
    '@/i18n/settingsText': { settingsText: translate },
    '@/lib/permissionConfig': permissionConfig,
    '@/lib/permissionProfiles': permissionProfiles,
    '@/lib/runtimeSettingLayer': { runtimeSettingBaseValue },
    './permissions/permissionNavigation': permissionNavigation,
    './settingsSessionScope': { resolveSettingsSessionId },
  }
  if (options.useRuntimeTransport) {
    const runtimeModule = { exports: {} }
    const runtimeModules: Record<string, unknown> = {
      '@/lib/api': modules['@/lib/api'],
      './modelConfigurationApi': { notifyModelConfigurationChanged() {} },
      './resourceSync': { captureResourceObservation: () => ({ scope: 1 }), invalidateResourcePrefix() {} },
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
  const panel = scope.run(() =>
    module.exports.default!.setup(props, {
      expose() {},
      emit(name: string, value: permissionConfig.PermissionConfig) {
        if (name === 'update:draftPermission') props.draftPermission = value
      },
    }),
  )!
  await panel.load()
  return {
    panel,
    props,
    writes,
    requests,
    reads,
    sessionWrites,
    messages,
    lastSessionId,
    directory,
    route,
    workspacePane,
    confirmations,
    setConfirmation(value: boolean) {
      confirmation = value
    },
    async navigate(view: string, scope = props.scope) {
      return await routeUpdate?.(
        { ...route, query: { ...route.query, view, scope } },
        { ...route, query: { ...route.query } },
      )
    },
    async guardRoute(to: unknown, from: unknown = { ...route, query: { ...route.query } }) {
      return await routeUpdate?.(to, from)
    },
    async guardLeave(to: unknown, from: unknown = { ...route, query: { ...route.query } }) {
      return await routeLeave?.(to, from)
    },
    async navigateSession(sessionId?: string) {
      const query = { ...route.query }
      if (sessionId) query.targetSessionId = sessionId
      else delete query.targetSessionId
      const allowed = await routeUpdate?.({ ...route, query }, { ...route, query: { ...route.query } })
      if (allowed === false) return false
      route.query = query
      props.sessionId = resolveSettingsSessionId(query.targetSessionId, lastSessionId.value || '1')
      await vue.nextTick()
      await Promise.resolve()
      await vue.nextTick()
      return true
    },
    setSessionPolicy(sessionId: string, policy: permissionConfig.PermissionConfig) {
      sessionPolicies.set(sessionId, structuredClone(policy))
    },
    holdSessionReads(sessionId: string) {
      let release!: () => void
      sessionReadGates.set(
        sessionId,
        new Promise<void>((resolve) => {
          release = resolve
        }),
      )
      return () => {
        sessionReadGates.delete(sessionId)
        release()
      }
    },
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
      route.query.scope = value
      await vue.nextTick()
      await Promise.resolve()
      await vue.nextTick()
    },
  }
}
