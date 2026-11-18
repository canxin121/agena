import { afterEach, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'
import * as agentProfiles from '../src/lib/agentProfiles'
import * as promptLayers from '../src/lib/promptLayers'
import * as promptLibrary from '../src/lib/promptLibrary'
import * as permissionConfig from '../src/lib/permissionConfig'
import * as modelSelectionDefaults from '../src/pages/chat/modelSelectionDefaults'

const source = readFileSync(new URL('../src/components/settings/AgentProfilesPanel.vue', import.meta.url), 'utf8')
const { descriptor } = parse(source)
const compiled = transformSync(compileScript(descriptor, { id: 'agent-profiles-test' }).content, {
  loader: 'ts',
  format: 'cjs',
}).code
const scopes: vue.EffectScope[] = []
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
})

async function editor(options: { scope?: string; globalSession?: boolean; missingProfile?: boolean } = {}) {
  const documents: Record<string, Record<string, any>> = {
    global: {
      profiles: { base: { title: 'Base' }, review: { title: 'Review', prompts: { order: ['style'] } } },
      default_profile: 'base',
      prompts: { library: [{ id: 'style', body: 'Use evidence' }] },
    },
    workspace: {
      profiles: { project: { title: 'Project', extends: 'base' }, review: { title: 'Project review' } },
      prompts: {},
    },
  }
  const writes: Array<{ path: string; value: any; options: any; layer: string }> = []
  const sessionWrites: Array<{ id: string; profile: string }> = []
  let sessionProfile = options.missingProfile ? 'removed' : 'review'
  let failSave = false
  let canLeave: (() => Promise<boolean>) | undefined
  let confirmations = 0
  const props = vue.reactive({ scope: options.scope || 'global', sessionId: '9' })
  const modules: Record<string, any> = {
    vue: { ...vue, onMounted() {}, onBeforeUnmount() {} },
    'vue-router': {
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
    '@/pages/chat/modelSelectionDefaults': modelSelectionDefaults,
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
    '@/i18n/settingsText': { settingsText: (text: string) => text },
    '@/lib/runtimeSettings': {
      async readRuntimeSettingSources(path: string) {
        const global = { value: structuredClone(documents.global![path]), revision: 'global-v1' }
        const workspace = { value: structuredClone(documents.workspace![path]), revision: 'workspace-v1' }
        return {
          global,
          workspace,
          file: workspace,
          effective: {
            value: path === 'profiles' ? { ...global.value, ...workspace.value } : (workspace.value ?? global.value),
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
        return { id: '9', workspace_id: options.globalSession ? null : 1 }
      },
      async getSessionExecution() {
        if (sessionProfile === 'removed') throw new Error('Unknown agent profile: removed')
        return { session: { workspace_id: options.globalSession ? null : 1 }, execution: { profile: sessionProfile } }
      },
      async patchSessionMetadata(id: string, patch: { profile: string }) {
        sessionWrites.push({ id, profile: patch.profile })
        sessionProfile = patch.profile
      },
    },
  }
  const module = { exports: {} as { default?: { setup: (props: unknown, context: unknown) => any } } }
  new Function('require', 'module', 'exports', compiled)(
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
  const panel = effectScope.run(() => module.exports.default!.setup(props, { expose() {} }))!
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
  panel.setPathPermission('workspace', 'readonly')
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

test('workspace defaults distinguish inheritance from an explicit no-profile default', async () => {
  const { panel, writes } = await editor({ scope: 'workspace' })
  panel.defaultName.value = ''
  await panel.saveDefault()
  expect(writes[0]).toMatchObject({ path: 'default_profile', value: '', layer: 'workspace' })
  panel.defaultName.value = '__inherit__'
  await panel.saveDefault()
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
  expect(Object.keys(panel.visibleCatalog.value).sort()).toEqual(['base', 'review'])
  expect(panel.visibleCatalog.value.review.title).toBe('Review')
})

test('missing global profiles can be replaced using only the global catalog', async () => {
  const { panel, sessionWrites } = await editor({ scope: 'session', globalSession: true, missingProfile: true })
  expect(panel.sessionReadFailed.value).toBe(true)
  expect(Object.keys(panel.visibleCatalog.value).sort()).toEqual(['base', 'review'])
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
  expect(Object.keys(panel.visibleCatalog.value).sort()).toEqual(['base', 'project', 'review'])
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

test('saving a profile keeps a separately edited default ready to save', async () => {
  const { panel, writes } = await editor()
  panel.draft.value.description = 'New description'
  panel.defaultName.value = 'review'
  await panel.saveProfile()
  expect(panel.defaultName.value).toBe('review')
  expect(panel.defaultDirty.value).toBe(true)
  expect(panel.dirty.value).toBe(false)
  await panel.saveDefault()
  expect(writes[1]).toMatchObject({ path: 'default_profile', value: 'review' })
})

test('invalid permission JSON cannot become an implicit permissive empty policy', async () => {
  const { panel, writes } = await editor()
  panel.permissionJson.value = '{"path":{"workspace":"read-write"}}'
  await panel.saveProfile()
  expect(writes).toEqual([])
  expect(panel.error.value).toContain('Invalid permission value')
  expect(panel.dirty.value).toBe(true)
})
