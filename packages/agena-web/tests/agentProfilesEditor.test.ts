import { afterEach, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'
import * as agentProfiles from '../src/lib/agentProfiles'
import * as promptLayers from '../src/lib/promptLayers'
import * as promptLibrary from '../src/lib/promptLibrary'
import * as permissionConfig from '../src/lib/permissionConfig'
import * as permissionProfiles from '../src/lib/permissionProfiles'
import { runtimeSettingBaseValue } from '../src/lib/runtimeSettingLayer'
import * as settingsDestination from '../src/components/settings/settingsDestination'

const source = readFileSync(new URL('../src/components/settings/AgentProfilesPanel.vue', import.meta.url), 'utf8')
const defaultsSource = readFileSync(
  new URL('../src/components/settings/AgentProfileDefaultsPanel.vue', import.meta.url),
  'utf8',
)
function compilePanel(source: string, id: string) {
  return transformSync(compileScript(parse(source).descriptor, { id }).content, { loader: 'ts', format: 'cjs' }).code
}
const compiled = compilePanel(source, 'agent-profiles-test')
const defaultsCompiled = compilePanel(defaultsSource, 'agent-profile-defaults-test')
const scopes: vue.EffectScope[] = []
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
})

async function editor(
  options: {
    scope?: string
    globalSession?: boolean
    missingProfile?: boolean
    defaultSession?: boolean
    subagent?: boolean
    emptyProfiles?: boolean
    editDefaults?: boolean
  } = {},
) {
  const builtins = {
    main: { title: 'Main', description: 'Coordinate the task', mode: 'primary' },
    explore: {
      title: 'Explore',
      description: 'Read and search',
      mode: 'subagent',
      permission: { path: { workspace: 'readonly' } },
    },
    worker: { title: 'Worker', description: 'Complete assigned work', mode: 'subagent' },
    reviewer: { title: 'Reviewer', mode: 'all' },
  }

  const documents: Record<string, Record<string, any>> = {
    global: {
      profiles: { base: { title: 'Base' }, review: { title: 'Review', prompts: { order: ['style'] } } },
      default_profile: 'base',
      default_subagent_profile: 'worker',
      prompts: { library: [{ id: 'style', body: 'Use evidence' }] },
    },
    workspace: {
      profiles: { project: { title: 'Project', extends: 'base' }, review: { title: 'Project review' } },
      prompts: {},
    },
  }
  if (options.emptyProfiles) {
    documents.global!.profiles = {}
    documents.workspace!.profiles = {}
    documents.global!.default_profile = undefined
  }
  const writes: Array<{ path: string; value: any; options: any; layer: string }> = []
  const sessionWrites: Array<{ id: string; profile: string }> = []
  let sessionProfile = options.missingProfile ? 'removed' : options.defaultSession ? '' : 'review'
  let failSave = false
  let canLeave: (() => Promise<boolean>) | undefined
  let confirmations = 0
  const props = vue.reactive({ scope: options.scope || 'global', sessionId: '9' })
  const modules: Record<string, any> = {
    vue: { ...vue, useId: () => 'fixture-id', onMounted() {}, onBeforeUnmount() {} },
    'vue-router': {
      RouterLink: {},
      useRoute: () => ({
        query: { view: 'agent-profiles', scope: options.scope || 'global', targetSessionId: '9', sessionId: 'retired' },
      }),
      onBeforeRouteLeave(fn: () => Promise<boolean>) {
        canLeave = fn
      },
      onBeforeRouteUpdate() {},
    },
    '@remixicon/vue': {},
    '@/lib/agentProfiles': agentProfiles,
    '@/lib/promptLayers': promptLayers,
    '@/lib/promptLibrary': promptLibrary,
    '@/lib/permissionConfig': permissionConfig,
    '@/lib/permissionProfiles': permissionProfiles,
    '@/lib/runtimeSettingLayer': { runtimeSettingBaseValue },
    '@/app/windowScope': { readWindowIdFromQuery: () => '' },
    '@/app/workspace/workspacePaneContext': { useWorkspacePaneContext: () => null },
    '@/pages/chat/modelSelectionCatalog': {
      useModelSelectionCatalog: () => ({ providers: vue.ref([]), async loadProvidersAndModels() {} }),
    },
    '@/lib/appConfirm': {
      async confirmAction() {
        confirmations++
        return true
      },
    },
    '@/stores/directory': { useDirectoryStore: () => vue.reactive({ currentDirectory: '/project' }) },
    './settingsSessionScope': { useSettingsSessionTarget: () => ({ sessionId: vue.ref('9') }) },
    './settingsDestination': settingsDestination,
    '@/i18n/settingsText': { settingsText: (text: string) => text },
    '@/lib/runtimeSettings': {
      subscribeRuntimeSettingsChanges: () => () => {},
      async readRuntimeSettingSources(path: string) {
        const global = { value: structuredClone(documents.global![path]), revision: 'global-v1' }
        const workspace = { value: structuredClone(documents.workspace![path]), revision: 'workspace-v1' }
        return {
          global,
          workspace,
          file: workspace,
          effective: {
            value:
              path === 'builtin_profiles'
                ? structuredClone(builtins)
                : path === 'profiles'
                  ? agentProfiles.withBuiltinAgentProfiles(agentProfiles.agentProfileCatalog(builtins), {
                      ...global.value,
                      ...workspace.value,
                    })
                  : (workspace.value ?? global.value),
          },
        }
      },
      async setRuntimeSetting(path: string, value: unknown, editOptions: unknown, layer: string) {
        if (failSave) throw new Error('settings revision changed')
        const copied = JSON.parse(JSON.stringify(value))
        writes.push({ path, value: copied, options: editOptions, layer })
        documents[layer]![path] = copied
        return { changed: true, current: copied }
      },
    },
    '@/stores/chat/api': {
      async getSession() {
        return {
          id: '9',
          workspace_id: options.globalSession ? null : 1,
          relation_kind: options.subagent ? 'subagent' : 'root',
        }
      },
      async getSessionExecution() {
        if (sessionProfile === 'removed') throw new Error('Unknown agent profile: removed')
        return {
          session: { workspace_id: options.globalSession ? null : 1 },
          execution: {
            profile: sessionProfile || (options.subagent ? 'worker' : 'main'),
            profile_is_default: !sessionProfile,
          },
        }
      },
      async patchSessionMetadata(id: string, patch: { profile: string }) {
        sessionWrites.push({ id, profile: patch.profile })
        sessionProfile = patch.profile
      },
    },
  }
  const module = { exports: {} as { default?: { setup: (props: unknown, context: unknown) => any } } }
  new Function('require', 'module', 'exports', options.editDefaults ? defaultsCompiled : compiled)(
    (id: string) => {
      if (id.endsWith('.vue')) return {}
      if (!(id in modules)) throw new Error(`Unexpected dependency: ${id}`)
      return modules[id]
    },
    module,
    module.exports,
  )
  const effectScope = vue.effectScope()
  scopes.push(effectScope)
  const panel = effectScope.run(() => module.exports.default!.setup(props, { expose() {}, emit() {} }))!
  await panel.load()
  return {
    panel,
    props,
    documents,
    writes,
    sessionWrites,
    rejectSaves() {
      failSave = true
    },
    canLeave: () => canLeave?.(),
    confirmations: () => confirmations,
  }
}

test('profile edits preserve other profiles and send the current layer revision', async () => {
  const { panel, writes } = await editor()
  panel.beginEditing('review', panel.catalog.value.review)
  panel.instructions.value = 'Check correctness'
  panel.permission.value = { path: { workspace: 'readonly' } }
  await panel.saveProfile()
  expect(panel.error.value).toBe('')
  expect(writes[0]?.path).toBe('profiles')
  expect(writes[0]?.layer).toBe('global')
  expect(writes[0]?.options).toEqual({ expected_revision: 'global-v1' })
  expect(writes[0]?.value.base).toEqual({ title: 'Base' })
  expect(writes[0]?.value.review.prompts.order).toEqual(['style', 'profile-review'])
  expect(writes[0]?.value.review.permission.path.workspace).toBe('readonly')
  expect(panel.dirty.value).toBe(false)
})

test('workspace defaults distinguish inheritance from an explicit built-in default', async () => {
  const { panel, writes } = await editor({ scope: 'workspace', editDefaults: true })
  panel.selectDefault('primary', '')
  await panel.saveDefault('primary')
  expect(writes[0]).toMatchObject({ path: 'default_profile', value: '', layer: 'workspace' })
  panel.selectDefault('primary', panel.inherit)
  await panel.saveDefault('primary')
  expect(writes[1]).toMatchObject({ path: 'default_profile', value: null, layer: 'workspace' })
})

test('a missing selected profile can be cleared even when execution state fails to load', async () => {
  const { panel, sessionWrites, canLeave, confirmations } = await editor({ scope: 'session', missingProfile: true })
  expect(panel.sources.value).not.toBeNull()
  expect(panel.error.value).toContain('removed')
  expect(panel.sessionReadFailed.value).toBe(true)
  expect(await canLeave()).toBe(true)
  expect(confirmations()).toBe(0)
  panel.sessionProfile.value = ''
  await panel.saveSession()
  expect(sessionWrites).toEqual([{ id: '9', profile: '' }])
  expect(panel.sessionReadFailed.value).toBe(false)
  expect(panel.error.value).toBe('')
})

test('global conversations show global profiles instead of the selected project catalog', async () => {
  const { panel } = await editor({ scope: 'session', globalSession: true })
  expect(Object.keys(panel.visibleCatalog.value).sort()).toEqual([
    'base',
    'explore',
    'main',
    'review',
    'reviewer',
    'worker',
  ])
  expect(panel.visibleCatalog.value.review.title).toBe('Review')
})

test('missing global profiles can be replaced using only the global catalog', async () => {
  const { panel, sessionWrites } = await editor({ scope: 'session', globalSession: true, missingProfile: true })
  expect(panel.sessionReadFailed.value).toBe(true)
  expect(Object.keys(panel.visibleCatalog.value).sort()).toEqual([
    'base',
    'explore',
    'main',
    'review',
    'reviewer',
    'worker',
  ])
  expect(panel.visibleCatalog.value.review.title).toBe('Review')
  panel.sessionProfile.value = 'base'
  await panel.saveSession()
  expect(sessionWrites).toEqual([{ id: '9', profile: 'base' }])
  expect(panel.sessionReadFailed.value).toBe(false)
  expect(panel.error.value).toBe('')
})

test('summary identity refreshes the catalog when missing-profile sessions change scopes', async () => {
  const options = { scope: 'session', globalSession: true, missingProfile: true }
  const { panel } = await editor(options)
  expect(panel.globalSession.value).toBe(true)
  options.globalSession = false
  await panel.load()
  expect(panel.sessionReadFailed.value).toBe(true)
  expect(panel.globalSession.value).toBe(false)
  expect(Object.keys(panel.visibleCatalog.value).sort()).toEqual([
    'base',
    'explore',
    'main',
    'project',
    'review',
    'reviewer',
    'worker',
  ])
  expect(panel.visibleCatalog.value.review.title).toBe('Project review')
})

test('revision conflicts preserve the unsaved profile for correction', async () => {
  const { panel, writes, rejectSaves } = await editor()
  panel.draft.value.description = 'New description'
  rejectSaves()
  await panel.saveProfile()
  expect(writes).toEqual([])
  expect(panel.error.value).toBe('settings revision changed')
  expect(panel.dirty.value).toBe(true)
  expect(panel.draft.value.description).toBe('New description')
})

test('refreshing session settings after a profile edit preserves the pending default', async () => {
  const { panel, documents, writes } = await editor({ editDefaults: true })
  panel.selectDefault('primary', 'review')
  documents.global!.profiles.review.title = 'Reviewed profile'
  await panel.refresh()
  expect(panel.rows.value[0]).toMatchObject({ selected: 'review', dirty: true })
  expect(panel.rows.value[0].options.review.title).toBe('Reviewed profile')
  await panel.saveDefault('primary')
  expect(writes[0]).toMatchObject({ path: 'default_profile', value: 'review' })
  expect(panel.dirty.value).toBe(false)
})

test('invalid permission edits cannot become an implicit permissive empty policy', async () => {
  const { panel, writes } = await editor()
  panel.permission.value = { path: { workspace: 'read-write' } }
  await panel.saveProfile()
  expect(writes).toEqual([])
  expect(panel.error.value).toContain('Invalid permission value')
  expect(panel.dirty.value).toBe(true)
})

test('empty user catalogs display usable builtins without serializing them into user config', async () => {
  const { panel, writes } = await editor({ emptyProfiles: true })
  expect(Object.keys(panel.catalog.value).sort()).toEqual(['explore', 'main', 'reviewer', 'worker'])
  expect(panel.profileSource('main')).toBe('Built-in')
  panel.beginEditing('main', panel.catalog.value.main)
  expect(panel.hasOwnProfile.value).toBe(false)
  panel.draft.value.description = 'Own the delivery'
  await panel.saveProfile()
  expect(panel.error.value).toBe('')
  expect(Object.keys(writes[0]?.value)).toEqual(['main'])
  expect(writes[0]?.value.main.mode).toBe('primary')
  expect(panel.profileSource('main')).toBe('Global')
  await panel.removeProfile()
  expect(writes[1]?.value).toEqual({})
  expect(panel.catalog.value.main.description).toBe('Coordinate the task')
})

test('workspace builtin overrides preserve only workspace entries and can restore the inherited role', async () => {
  const { panel, writes } = await editor({ scope: 'workspace' })
  panel.beginEditing('explore', panel.catalog.value.explore)
  expect(panel.inheritedPermission.value.path.workspace).toBe('readonly')
  expect(panel.permission.value).toEqual({})
  panel.draft.value.hidden = true
  await panel.saveProfile()
  expect(panel.error.value).toBe('')
  expect(Object.keys(writes[0]?.value).sort()).toEqual(['explore', 'project', 'review'])
  expect(writes[0]?.value.explore.hidden).toBe(true)
  expect(writes[0]?.value.explore.permission).toEqual({})
  expect(panel.profileSource('explore')).toBe('Workspace')
  await panel.removeProfile()
  expect(panel.catalog.value.explore.hidden).toBeUndefined()
  expect(panel.profileSource('explore')).toBe('Built-in')
})

test('root and delegated session selectors filter inherited mode while allowing hidden roles', async () => {
  const { panel, documents, sessionWrites } = await editor({ scope: 'session' })
  documents.global!.profiles.private = { extends: 'explore', hidden: true }
  await panel.load()
  expect(panel.sessionCatalog.value.main).toBeDefined()
  expect(panel.sessionCatalog.value.explore).toBeUndefined()
  panel.sessionProfile.value = 'explore'
  await panel.saveSession()
  expect(sessionWrites).toEqual([])
  expect(panel.error.value).toContain('compatible')
  const child = await editor({ scope: 'session', subagent: true })
  child.documents.global!.profiles.private = { extends: 'explore', hidden: true }
  await child.panel.load()
  expect(child.panel.sessionCatalog.value.main).toBeUndefined()
  expect(child.panel.sessionCatalog.value.private).toBeDefined()
  child.panel.sessionProfile.value = 'private'
  await child.panel.saveSession()
  expect(child.sessionWrites).toEqual([{ id: '9', profile: 'private' }])
})

test('editing one profile permission leaves its inherited rules at the correct source', async () => {
  const { panel, documents, writes } = await editor({ scope: 'workspace' })
  documents.global!.profiles.base.permission = {
    network: { version: 2, default: 'deny', allow: [{ to: '192.0.2.10' }] },
    tools: { names: { 'agena.fs.read': 'allow' } },
  }
  documents.workspace!.profiles.project.permission = { network: { version: 2, allow: [] } }
  await panel.load('project')
  expect(panel.inheritedPermission.value.network).toEqual({
    version: 2,
    default: 'deny',
    allow: [{ to: '192.0.2.10' }],
  })
  panel.permission.value.tools = { default: 'ask' }
  await panel.saveProfile()
  expect(writes[0]?.value.project.permission).toEqual({
    network: { version: 2, allow: [] },
    tools: { default: 'ask' },
  })
  expect(writes[0]?.value.base).toBeUndefined()
})

test('default session state shows its effective role and clearing a manual role restores it', async () => {
  const { panel, sessionWrites } = await editor({ scope: 'session', defaultSession: true })
  expect(panel.sessionProfile.value).toBe('')
  expect(panel.effectiveSessionProfile.value).toBe('main')
  expect(panel.dirty.value).toBe(false)
  panel.sessionProfile.value = 'reviewer'
  await panel.saveSession()
  panel.sessionProfile.value = ''
  await panel.saveSession()
  expect(sessionWrites.at(-1)).toEqual({ id: '9', profile: '' })
  expect(panel.effectiveSessionProfile.value).toBe('main')
  const child = await editor({ scope: 'session', defaultSession: true, subagent: true })
  expect(child.panel.effectiveSessionProfile.value).toBe('worker')
})

test('subagent defaults are saved separately with revision protection and compatible modes', async () => {
  const { panel, writes } = await editor({ scope: 'workspace', editDefaults: true })
  panel.selectDefault('subagent', 'explore')
  await panel.saveDefault('subagent')
  expect(writes[0]).toMatchObject({
    path: 'default_subagent_profile',
    value: 'explore',
    layer: 'workspace',
    options: { expected_revision: 'workspace-v1' },
  })
  panel.selectDefault('subagent', 'main')
  await panel.saveDefault('subagent')
  expect(writes).toHaveLength(1)
  panel.selectDefault('primary', 'explore')
  await panel.saveDefault('primary')
  expect(writes).toHaveLength(1)
})

test('inherited mode and visibility survive editing and invalid metadata keeps the draft', async () => {
  const { panel, writes } = await editor()
  panel.beginEditing('custom', { extends: 'explore', hidden: true }, true)
  await panel.saveProfile()
  expect(writes[0]?.value.custom).toMatchObject({ extends: 'explore', hidden: true })
  panel.draft.value.mode = 'invalid'
  await panel.saveProfile()
  expect(writes).toHaveLength(1)
  expect(panel.error.value).toContain('Invalid profile mode')
  expect(panel.dirty.value).toBe(true)
})

test('partial raw built-in settings render inherited behavior in global and workspace editors', async () => {
  const { panel, documents } = await editor()
  documents.global!.profiles.explore = { model: { model: 'fast' } }
  await panel.load('explore')
  expect(panel.catalog.value.explore.mode).toBe('subagent')
  expect(panel.catalog.value.explore.description).toBe('Read and search')
  expect(panel.catalog.value.explore.permission.path.workspace).toBe('readonly')
  expect(panel.profileSource('explore')).toBe('Global')
})

test('default summaries show persisted values and their scope, independently of unsaved selections', async () => {
  const { panel, documents } = await editor({ editDefaults: true })
  const primary = () => panel.rows.value[0]
  const subagent = () => panel.rows.value[1]
  documents.workspace!.default_profile = 'project'
  documents.workspace!.default_subagent_profile = 'explore'
  await panel.load()
  expect(primary()).toMatchObject({ name: 'base', source: 'Global', inherited: 'main' })
  expect(subagent()).toMatchObject({ name: 'worker', source: 'Global' })
  panel.selectDefault('primary', 'reviewer')
  expect(primary().name).toBe('base')
  await panel.saveDefault('primary')
  expect(primary()).toMatchObject({ name: 'reviewer', source: 'Global' })
  documents.global!.default_profile = ''
  documents.global!.default_subagent_profile = null
  await panel.load()
  expect(primary()).toMatchObject({ name: 'main', source: 'Global' })
  expect(subagent()).toMatchObject({ name: 'worker', source: 'Built-in' })
})

test('workspace default summaries resolve inherited, explicit and restored values after saving', async () => {
  const { panel, writes } = await editor({ scope: 'workspace', editDefaults: true })
  const primary = () => panel.rows.value[0]
  const subagent = () => panel.rows.value[1]
  expect(primary()).toMatchObject({ name: 'base', source: 'Global', inherited: 'base' })
  panel.selectDefault('primary', '')
  panel.selectDefault('subagent', 'explore')
  await panel.saveDefault('primary')
  expect(primary()).toMatchObject({ name: 'main', source: 'Workspace' })
  expect(subagent().selected).toBe('explore')
  await panel.saveDefault('subagent')
  expect(subagent()).toMatchObject({ name: 'explore', source: 'Workspace' })
  panel.selectDefault('primary', panel.inherit)
  await panel.saveDefault('primary')
  expect(primary()).toMatchObject({ name: 'base', source: 'Global' })
  expect(writes.every((write) => write.options.expected_revision === 'workspace-v1')).toBe(true)
})

test('session scope links directly to both default configuration layers while preserving the chosen session', async () => {
  const { panel } = await editor({ scope: 'session', missingProfile: true })
  for (const scope of ['global', 'workspace'] as const) {
    expect(panel.defaultSettingsDestination(scope)).toEqual({
      path: '/settings/runtime-session',
      query: { view: 'session-behavior', scope, targetSessionId: '9' },
      hash: '#settings-default-agent-profiles',
    })
  }
  expect(source).not.toContain('data-testid="agent-profile-defaults"')
  expect(defaultsSource).toContain('data-testid="agent-profile-defaults"')
})
