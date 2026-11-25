import { setupSettingsDraftGuard } from './helpers/setupSettingsDraftGuard'
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, compileTemplate, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'
import { hasRuntimeLayerValue, runtimeSettingBaseValue } from '../src/lib/runtimeSettingLayer'
import { encodeModelSelectionKey, parseModelSlug } from '../src/pages/chat/modelSelectionDefaults'

const scopes: vue.EffectScope[] = []
const disposers: Array<() => void> = []
afterEach(() => {
  for (const dispose of disposers.splice(0)) dispose()
  for (const scope of scopes.splice(0)) scope.stop()
})
const scripts = new Map<string, string>()
const renders = new Map<string, (...args: any[]) => any>()
function templateNodes(name: string, panel: any, props: Record<string, unknown>) {
  if (!renders.has(name)) {
    const descriptor = parse(
      readFileSync(new URL(`../src/components/settings/${name}.vue`, import.meta.url), 'utf8'),
    ).descriptor
    const bindings = compileScript(descriptor, { id: name }).bindings
    const template = compileTemplate({
      source: descriptor.template!.content,
      filename: `${name}.vue`,
      id: name,
      compilerOptions: { bindingMetadata: bindings },
    })
    const compiled = transformSync(template.code, { loader: 'ts', format: 'cjs' }).code
    const module = { exports: {} as any }
    new Function('require', 'module', 'exports', compiled)(
      (id: string) => (id === 'vue' ? vue : {}),
      module,
      module.exports,
    )
    renders.set(name, module.exports.render)
  }
  const nodes: any[] = []
  const visit = (node: any) => {
    if (!node || typeof node !== 'object') return
    nodes.push(node)
    const children = Array.isArray(node.children) ? node.children : node.children?.default?.() || []
    for (const child of children) visit(child)
  }
  const setupState = {
    ...vue.proxyRefs(panel),
    RiCpuLine: {},
    RiRefreshLine: {},
    RiCodeSSlashLine: {},
    RiShieldCheckLine: {},
  }
  visit(renders.get(name)!({ $st: (key: string) => key }, [], props, setupState))
  return nodes
}

function setup(name: string, props: Record<string, unknown>, modules: Record<string, unknown>) {
  if (!scripts.has(name)) {
    const file = readFileSync(new URL(`../src/components/settings/${name}.vue`, import.meta.url), 'utf8')
    scripts.set(
      name,
      transformSync(compileScript(parse(file).descriptor, { id: name }).content, { loader: 'ts', format: 'cjs' }).code,
    )
  }
  const module = { exports: {} as { default: { setup: (props: unknown, ctx: unknown) => any } } }
  const dependencies: Record<string, unknown> = {
    vue: { ...vue, onMounted() {}, onBeforeUnmount: (fn: () => void) => disposers.push(fn) },
    'vue-router': { onBeforeRouteUpdate() {}, onBeforeRouteLeave() {} },
    '@remixicon/vue': {},
    '@/i18n/settingsText': { settingsText: (key: string) => key },
    '@/stores/toasts': { useToastsStore: () => ({ push() {} }) },
    '@/lib/appConfirm': { confirmAction: async () => true },
    '@/lib/runtimeSettingLayer': { hasRuntimeLayerValue, runtimeSettingBaseValue },
    '@/lib/permissionConfig': { parsePermissionConfig: (v: unknown) => v, permissionConfigErrorMessage: String },
    ...modules,
  }
  setupSettingsDraftGuard(dependencies)
  new Function('require', 'module', 'exports', scripts.get(name)!)(
    (id: string) => {
      if (id.endsWith('.vue')) return {}
      if (!(id in dependencies)) throw new Error('Unexpected dependency: ' + id)
      return dependencies[id]
    },
    module,
    module.exports,
  )
  const scope = vue.effectScope()
  scopes.push(scope)
  return scope.run(() => module.exports.default.setup(props, { expose() {} }))
}

function sources(
  global: unknown = undefined,
  workspace: unknown = undefined,
  parent: unknown = undefined,
  effective: unknown = undefined,
) {
  return {
    global: { config_found: true, value: global, inherited_value: parent },
    workspace: { config_found: true, value: workspace, inherited_value: parent ?? global },
    effective: { value: effective ?? workspace ?? global ?? parent },
    file: { value: global },
  }
}
function transport(initial: ReturnType<typeof sources>) {
  const state = structuredClone(initial)
  const writes: Array<{ action: string; path: string; value?: unknown; options: any; layer: string }> = []
  let failure = ''
  return {
    state,
    writes,
    fail(message: string) {
      failure = message
    },
    api: {
      hasPersistedSetting: hasRuntimeLayerValue,
      async readRuntimeSettingSources() {
        return structuredClone(state)
      },
      async setRuntimeSetting(path: string, value: unknown, options: any, layer: 'global' | 'workspace') {
        if (failure) throw new Error(failure)
        writes.push({ action: 'set', path, value, options, layer })
        if (!options.dry_run) Object.assign(state[layer], { value: structuredClone(value), has_override: true })
        return {}
      },
      async deleteRuntimeSetting(path: string, options: any, layer: 'global' | 'workspace') {
        if (failure) throw new Error(failure)
        writes.push({ action: 'delete', path, options, layer })
        if (!options.dry_run)
          Object.assign(state[layer], { value: undefined, has_override: false, override_path: undefined })
        return {}
      },
    },
  }
}

const parent = { provider: 'p', model: 'parent', thinking_mode: 'low', speed_mode: 'normal' }
const local = { provider: 'p', model: 'local', thinking_mode: 'high' }
const later = { provider: 'p', model: 'later' }
const key = encodeModelSelectionKey

async function modelEditor({
  layer = 'session',
  selected = {},
  inherited = parent,
  effective = later,
  bundle = sources(),
  legacy = false,
}: {
  layer?: string
  selected?: any
  inherited?: any
  effective?: any
  bundle?: ReturnType<typeof sources>
  legacy?: boolean
} = {}) {
  const wire = transport(bundle)
  const props = vue.reactive({ scope: layer, purpose: 'default', sessionId: '10' })
  let state: any = {
    execution: {
      ...(!legacy ? { selected_selection: selected, inherited_selection: inherited } : {}),
      model_provider_id: effective.provider,
      model_id: effective.model,
      model_thinking_mode: effective.thinking_mode,
    },
  }
  const sessionWrites: Array<{ id: string; options: any }> = []
  let waitForWrite: Promise<void> | undefined
  const modes = [
    { value: 'low', label: 'Low', isDefault: true },
    { value: 'high', label: 'High', isDefault: false },
  ]
  const panel = setup('ModelSelectionSettings', props, {
    '@/lib/runtimeSettings': wire.api,
    '@/lib/serverModelSettings': { sameServerModelIdentity: (a: any, b: any) => key(a || {}) === key(b || {}) },
    '@/pages/chat/modelSelectionDefaults': { encodeModelSelectionKey, parseModelSlug },
    '@/pages/chat/modelSelectionCatalog': {
      useModelSelectionCatalog: () => ({
        providers: vue.ref([{ id: 'p', models: ['parent', 'local', 'later'].map((id) => ({ id })) }]),
        catalogError: vue.ref(''),
        modelMetaFor: () => ({}),
        async loadProvidersAndModels() {},
      }),
      defaultModeValue: () => 'low',
      thinkingModeOptionsForModel: () => modes,
      speedModeOptionsForModel: () => modes,
      verbosityOptionsForModel: () => modes,
    },
    '@/stores/chat/api': {
      async getSessionExecution() {
        return structuredClone(state)
      },
      async setSessionSelection(id: string, options: any) {
        const currentState = state
        await waitForWrite
        sessionWrites.push({ id, options })
        let raw: any = options.model
          ? {
              provider: options.model.provider_id,
              model: options.model.model_id,
              ...(options.model.adapter_id ? { adapter: options.model.adapter_id } : {}),
            }
          : { ...currentState.execution.selected_selection }
        if (!Object.keys(options).length) raw = {}
        for (const mode of ['thinking_mode', 'speed_mode', 'verbosity']) {
          if (options[mode]) raw[mode] = options[mode]
          else if (mode in options) delete raw[mode]
        }
        if (typeof options.parallel_tool_calls === 'boolean') raw.parallel_tool_calls = options.parallel_tool_calls
        currentState.execution.selected_selection = raw
        return structuredClone(currentState)
      },
    },
  })
  await panel.refresh()
  return {
    panel,
    props,
    sessionWrites,
    ...wire,
    setSessionState(next: any) {
      state = next
    },
    wait(p: Promise<void>) {
      waitForWrite = p
    },
  }
}

describe('model settings distinguish persisted selection from inherited selection', () => {
  test('session model pinning keeps an inherited parallel-tool disable explicit', async () => {
    const { panel, sessionWrites } = await modelEditor({ inherited: { ...parent, parallel_tool_calls: false } })
    panel.chooseSource('local')
    await panel.save()
    expect(sessionWrites[0]?.options.parallel_tool_calls).toBe(false)
    expect(panel.parallelToolCalls.value).toBe(false)
    panel.chooseSource('inherit')
    await panel.save()
    expect(sessionWrites[1]?.options).toEqual({})
  })

  test('an in-flight session save cannot overwrite the newly selected conversation editor', async () => {
    const { panel, props, sessionWrites, wait, setSessionState } = await modelEditor()
    let release!: () => void
    wait(
      new Promise<void>((resolve) => {
        release = resolve
      }),
    )
    panel.chooseSource('local')
    const pending = panel.save()
    props.sessionId = '11'
    setSessionState({
      execution: {
        selected_selection: {},
        inherited_selection: later,
        model_provider_id: 'p',
        model_id: 'later',
      },
    })
    await vue.nextTick()
    release()
    await pending
    await panel.refresh()
    expect(sessionWrites[0]?.id).toBe('10')
    expect(panel.sourceMode.value).toBe('inherit')
    expect(panel.displayedModelKey.value).toBe(key(later))
    expect(panel.dirty.value).toBe(false)
  })

  test('session model comes from the parent when the raw session selection is empty', async () => {
    const { panel } = await modelEditor()
    expect(panel.sourceMode.value).toBe('inherit')
    expect(panel.persisted.value).toBe(false)
    expect(panel.modelKey.value).toBe('')
    expect(panel.displayedModelKey.value).toBe(key(parent))
    expect(panel.preferenceValue('thinking_mode')).toBe('low')
  })

  test('pinning an inherited session model saves its route even when the visible value stays equal', async () => {
    const { panel, sessionWrites } = await modelEditor()
    panel.chooseSource('local')
    expect(panel.dirty.value).toBe(true)
    expect(panel.displayedModelKey.value).toBe(key(parent))
    await panel.save()
    expect(sessionWrites[0]).toEqual({
      id: '10',
      options: {
        model: { provider_id: 'p', model_id: 'parent' },
        thinking_mode: 'low',
        speed_mode: 'normal',
      },
    })
    expect(panel.sourceMode.value).toBe('local')
    expect(panel.dirty.value).toBe(false)
  })

  test('restoring session inheritance submits an empty selection and displays its parent', async () => {
    const { panel, sessionWrites } = await modelEditor({ selected: local })
    panel.chooseSource('inherit')
    expect(panel.displayedModelKey.value).toBe(key(parent))
    await panel.save()
    expect(sessionWrites).toEqual([{ id: '10', options: {} }])
    expect(panel.sourceMode.value).toBe('inherit')
  })

  test('a session mode-only override cannot unlock the inherited route or other modes', async () => {
    const { panel, sessionWrites } = await modelEditor({
      selected: { thinking_mode: 'high', parallel_tool_calls: false },
    })
    expect(panel.modelSource.value).toBe('inherit')
    expect(panel.modelValueDisabled.value).toBe(true)
    expect(panel.preferenceValueDisabled('thinking_mode')).toBe(false)
    expect(panel.preferenceValueDisabled('speed_mode')).toBe(true)
    panel.chooseModel(key(local))
    panel.choosePreference('speed_mode', 'high')
    await panel.save()
    expect(sessionWrites).toEqual([])
    expect(panel.modelKey.value).toBe('')
    panel.chooseSource('local')
    expect(panel.modelValueDisabled.value).toBe(false)
    expect(panel.thinkingMode.value).toBe('high')
    expect(panel.parallelToolCalls.value).toBe(false)
    await panel.save()
    expect(sessionWrites[0]?.options).toMatchObject({
      model: { provider_id: 'p', model_id: 'parent' },
      thinking_mode: 'high',
      speed_mode: 'normal',
      parallel_tool_calls: false,
    })
  })

  test('rendered session controls keep inherited fields disabled and expose the actual parent until explicitly switched', async () => {
    const { panel, props, sessionWrites } = await modelEditor({ selected: { thinking_mode: 'high' } })
    const nodes = () => templateNodes('ModelSelectionSettings', panel, props)
    const picker = (title: string) => nodes().find((node) => node.props?.title === title && 'options' in node.props)
    const source = (label: string) => nodes().find((node) => node.props?.label === label && 'scope' in node.props)
    expect(picker('Default model').props.disabled).toBe(true)
    expect(picker('Default model').props['model-value']).toBe(key(parent))
    expect(picker('Thinking').props.disabled).toBe(false)
    expect(picker('Thinking').props['model-value']).toBe('high')
    expect(picker('Speed').props.disabled).toBe(true)
    expect(picker('Speed').props['model-value']).toBe('normal')
    expect(source('Speed').props['inherited-source']).toBe('Parent configuration')
    expect(source('Speed').props['inherited-value']).toBe('normal')
    picker('Default model').props['onUpdate:modelValue'](key(local))
    picker('Speed').props['onUpdate:modelValue']('high')
    await panel.save()
    expect(sessionWrites).toEqual([])
    source('Speed').props['onUpdate:modelValue']('local')
    expect(picker('Speed').props.disabled).toBe(false)
    expect(picker('Default model').props.disabled).toBe(true)
    expect(picker('Verbosity').props.disabled).toBe(true)
    picker('Speed').props['onUpdate:modelValue']('high')
    await panel.save()
    expect(sessionWrites[0]?.options).toEqual({ thinking_mode: 'high', speed_mode: 'high' })
  })

  test('a local session route owns its model-default response modes through the runtime cascade', async () => {
    const { panel } = await modelEditor({ selected: { provider: 'p', model: 'local' } })
    expect(panel.modelSource.value).toBe('local')
    expect(panel.preferenceSource('thinking_mode')).toBe('local')
    expect(panel.preferenceValueDisabled('thinking_mode')).toBe(false)
    expect(panel.preferenceValue('thinking_mode')).toBe('')
    expect(panel.preferenceEmptyLabel('thinking_mode')).toBe('Model default')
  })

  test('an unknown session parent permits an explicit mode draft without fabricating a route', async () => {
    const { panel, sessionWrites } = await modelEditor()
    panel.acceptSessionState({ execution: { selected_selection: {} } })
    panel.syncEditor()
    expect(panel.preferenceValueDisabled('verbosity')).toBe(true)
    expect(panel.inheritedPreferenceLabel('verbosity')).toBe('Unavailable')
    panel.choosePreferenceSource('verbosity', 'local')
    expect(panel.preferenceValueDisabled('verbosity')).toBe(false)
    await panel.save()
    expect(sessionWrites).toEqual([])
    panel.choosePreference('verbosity', 'high')
    await panel.save()
    expect(sessionWrites[0]?.options).toEqual({ verbosity: 'high' })
  })

  test('model drafts survive a pending inheritance reset and restore their local modes', async () => {
    const { panel, writes } = await modelEditor({ layer: 'workspace', bundle: sources(parent, local, parent) })
    panel.choosePreference('speed_mode', 'high')
    panel.chooseSource('inherit')
    expect(panel.modelValueDisabled.value).toBe(true)
    expect(panel.preferenceValue('speed_mode')).toBe('normal')
    panel.choosePreference('speed_mode', 'low')
    panel.chooseSource('local')
    expect(panel.speedMode.value).toBe('high')
    await panel.save()
    expect(writes[0]?.value).toEqual({ ...local, speed_mode: 'high' })
  })

  test('session mode-only editing retains the inherited route and does not pin unrelated modes', async () => {
    const { panel, sessionWrites } = await modelEditor()
    expect(panel.preferenceValueDisabled('verbosity')).toBe(true)
    panel.choosePreference('verbosity', 'high')
    expect(panel.dirty.value).toBe(false)
    panel.choosePreferenceSource('verbosity', 'local')
    panel.choosePreference('verbosity', 'high')
    await panel.save()
    expect(sessionWrites).toEqual([{ id: '10', options: { verbosity: 'high' } }])
    expect(panel.modelKey.value).toBe('')
    expect(panel.displayedModelKey.value).toBe(key(parent))
    expect(panel.sourceMode.value).toBe('local')
  })

  test('existing session mode-only overrides are shown as local and can clear one mode', async () => {
    const { panel, sessionWrites } = await modelEditor({ selected: { thinking_mode: 'high', verbosity: 'high' } })
    expect(panel.sourceMode.value).toBe('local')
    expect(panel.modelSource.value).toBe('inherit')
    expect(panel.modelValueDisabled.value).toBe(true)
    expect(panel.preferenceValueDisabled('thinking_mode')).toBe(false)
    expect(panel.preferenceValueDisabled('speed_mode')).toBe(true)
    panel.choosePreferenceSource('thinking_mode', 'inherit')
    await panel.save()
    expect(sessionWrites[0]?.options).toEqual({ thinking_mode: '', verbosity: 'high' })
    expect(panel.modelKey.value).toBe('')
  })

  test('older server effective fields are not claimed as local or used for a reset or pin', async () => {
    const { panel, sessionWrites } = await modelEditor({ legacy: true })
    expect(panel.sourceKnown.value).toBe(false)
    expect(panel.persisted.value).toBe(false)
    expect(panel.displayedModelKey.value).toBe('')
    panel.chooseSource('local')
    panel.chooseSource('inherit')
    panel.chooseModel('')
    await panel.save()
    expect(sessionWrites).toHaveLength(0)
    panel.chooseSource('local')
    panel.chooseModel(key(local))
    expect(panel.dirty.value).toBe(true)
  })

  test('global model editing shows the global raw group before later workspace overrides', async () => {
    const { panel } = await modelEditor({ layer: 'global', bundle: sources(local, later, parent, later) })
    expect(panel.displayedModelKey.value).toBe(key(local))
    expect(panel.sourceMode.value).toBe('local')
  })

  test('a global model uses the resolved current layer without copying unrelated builtin defaults', async () => {
    const resolved = { ...local, parallel_tool_calls: false }
    const bundle = sources(undefined, later, parent, later)
    Object.assign(bundle.global, { default_value: null, layer_value: resolved, inherited_value: undefined })
    const { panel, writes } = await modelEditor({ layer: 'global', bundle })
    expect(panel.displayedModelKey.value).toBe(key(local))
    expect(panel.preferenceValue('thinking_mode')).toBe('high')
    expect(panel.inheritedModelKey.value).toBe('')
    expect(panel.dirty.value).toBe(false)
    await panel.save()
    expect(writes).toEqual([])
    panel.choosePreference('verbosity', 'high')
    await panel.save()
    expect(writes[0]?.value).toEqual({ ...resolved, verbosity: 'high' })
  })

  test('a partial local file model remains atomic under the new resolved-value contract', async () => {
    const own = { provider: 'p', model: 'local' }
    const bundle = sources(parent, own, { ...parent, parallel_tool_calls: false }, later)
    Object.assign(bundle.workspace, { layer_value: own, default_value: null, has_override: true })
    const { panel, writes } = await modelEditor({ layer: 'workspace', bundle })
    expect(panel.displayedModelKey.value).toBe(key(own))
    expect(panel.preferenceValue('thinking_mode')).toBe('')
    panel.choosePreference('verbosity', 'high')
    await panel.save()
    expect(writes[0]?.value).toEqual({ ...own, verbosity: 'high' })
  })

  test('a known empty global model does not fall back to legacy or runtime models', async () => {
    const bundle = sources(undefined, later, parent, later)
    Object.assign(bundle.global, { default_value: null, layer_value: null })
    const { panel, writes } = await modelEditor({ layer: 'global', bundle })
    expect(panel.displayedModelKey.value).toBe('')
    expect(panel.inheritedModelKey.value).toBe('')
    expect(panel.dirty.value).toBe(false)
    await panel.save()
    expect(writes).toEqual([])
  })

  test('global models edit directly and reset the group without a source toggle', async () => {
    const defaults = { ...parent, parallel_tool_calls: false }
    const { panel, writes } = await modelEditor({
      layer: 'global',
      bundle: sources(undefined, later, defaults, later),
    })
    expect(panel.displayedModelKey.value).toBe(key(parent))
    expect(panel.dirty.value).toBe(false)
    panel.chooseSource('local')
    await panel.save()
    expect(writes).toEqual([])
    panel.choosePreference('verbosity', 'high')
    await panel.save()
    expect(writes[0]).toMatchObject({
      action: 'set',
      layer: 'global',
      value: { ...defaults, verbosity: 'high' },
    })
    panel.chooseSource('inherit')
    expect(panel.resetPending.value).toBe(true)
    await panel.save()
    expect(writes[1]).toMatchObject({ action: 'delete', layer: 'global', path: 'providers.default_selection' })
    expect(panel.displayedModelKey.value).toBe(key(parent))
  })

  test('file model settings pin the complete parent group, retaining false parallel preference', async () => {
    const { panel, writes, state } = await modelEditor({
      layer: 'workspace',
      bundle: sources(parent, undefined, { ...parent, parallel_tool_calls: false }, later),
    })
    panel.chooseSource('local')
    await panel.save()
    expect(writes[0]?.value).toEqual({ ...parent, parallel_tool_calls: false })
    panel.chooseSource('inherit')
    await panel.save()
    expect(writes[1]).toMatchObject({ action: 'delete', layer: 'workspace', path: 'providers.default_selection' })
    expect(state.global.value).toEqual(parent)
  })

  test('changing a file response mode promotes the whole model group as required by config replacement', async () => {
    const { panel, writes } = await modelEditor({ layer: 'workspace', bundle: sources(parent, undefined, parent) })
    expect(panel.modelValueDisabled.value).toBe(true)
    panel.choosePreference('verbosity', 'high')
    expect(panel.dirty.value).toBe(false)
    panel.chooseSource('local')
    panel.choosePreference('verbosity', 'high')
    await panel.save()
    expect(writes[0]?.value).toEqual({ ...parent, verbosity: 'high' })
  })
})

async function advancedEditor(bundle: ReturnType<typeof sources>) {
  const wire = transport(bundle)
  const props = vue.reactive({ scope: 'global' })
  let confirmResult: boolean | Promise<boolean> = true
  const confirmations: unknown[] = []
  const panel = setup('AdvancedSettingsPanel', props, {
    '@/lib/runtimeSettings': wire.api,
    '@/lib/appConfirm': {
      async confirmAction(options: unknown) {
        confirmations.push(options)
        return confirmResult
      },
    },
  })
  panel.settingPath.value = 'ui.example'
  await panel.load()
  return {
    panel,
    props,
    ...wire,
    confirmations,
    validationButton() {
      const node = templateNodes('AdvancedSettingsPanel', panel, props).find(
        (node) => node.props?.onClick === panel.validate,
      )
      if (!node) throw new Error('Advanced settings validation button was not rendered')
      return node
    },
    confirm(value: boolean | Promise<boolean>) {
      confirmResult = value
    },
  }
}

describe('advanced JSON editing uses explicit source operations', () => {
  test.each([[{ enabled: false, nested: { count: 0 } }], [false], [0], ['']])(
    'inherited JSON %p is visible and greyed out until the local source is selected',
    async (parent) => {
      const { panel, props, writes } = await advancedEditor(sources(parent, undefined, parent, { later: true }))
      props.scope = 'workspace'
      await vue.nextTick()
      const textarea = () =>
        templateNodes('AdvancedSettingsPanel', panel, props).find((node) => node.props?.id === 'advanced-setting-draft')
      expect(textarea().props.disabled).toBe(true)
      expect(JSON.parse(textarea().props.value)).toEqual(parent)
      expect(panel.valueDisabled.value).toBe(true)
      panel.editDraft({ target: { value: '{"local":false}' } })
      await panel.save()
      expect(writes).toEqual([])
      expect(panel.dirty.value).toBe(false)
      await panel.chooseSource('local')
      expect(textarea().props.disabled).toBe(false)
      expect(JSON.parse(panel.draftText.value)).toEqual(parent)
      panel.editDraft({ target: { value: '{"local":false}' } })
      await panel.save()
      expect(writes[0]).toMatchObject({ action: 'set', layer: 'workspace', value: { local: false } })
    },
  )

  test('restoring inherited JSON and cancelling that reset preserves the unsaved raw local document', async () => {
    const parent = { parent: true }
    const { panel, props, writes } = await advancedEditor(sources(parent, { local: false }, parent))
    props.scope = 'workspace'
    await vue.nextTick()
    panel.editDraft({ target: { value: '{"local":true,"count":0}' } })
    await panel.chooseSource('inherit')
    expect(JSON.parse(panel.displayedDraftText.value)).toEqual(parent)
    expect(panel.valueDisabled.value).toBe(true)
    panel.editDraft({ target: { value: '{"ignored":true}' } })
    await panel.chooseSource('local')
    expect(JSON.parse(panel.draftText.value)).toEqual({ local: true, count: 0 })
    await panel.save()
    expect(writes).toHaveLength(1)
    expect(writes[0]).toMatchObject({ action: 'set', layer: 'workspace', value: { local: true, count: 0 } })
  })

  test.each([[undefined], [null]])(
    'an unavailable or empty JSON parent %p supports a manual local draft without saving a synthetic null',
    async (parent) => {
      const bundle = sources()
      bundle.workspace.inherited_value = parent
      const { panel, props, writes, validationButton } = await advancedEditor(bundle)
      props.scope = 'workspace'
      await vue.nextTick()
      expect(panel.valueDisabled.value).toBe(true)
      expect(panel.displayedDraftText.value).toBe(parent === null ? 'null' : 'Unavailable')
      await panel.chooseSource('local')
      expect(panel.valueDisabled.value).toBe(false)
      expect(panel.draftText.value).toBe('')
      expect(validationButton().props.disabled).toBe(true)
      await panel.save()
      expect(writes).toEqual([])
      panel.editDraft({ target: { value: 'false' } })
      await panel.save()
      expect(writes[0]).toMatchObject({ action: 'set', layer: 'workspace', value: false })
    },
  )

  test('the effective JSON document is displayed read-only and never targets the global file for writes', async () => {
    const { panel, props, writes, validationButton } = await advancedEditor(
      sources({ global: true }, undefined, {}, { effective: true }),
    )
    props.scope = 'effective'
    await vue.nextTick()
    expect(JSON.parse(panel.displayedDraftText.value)).toEqual({ effective: true })
    expect(panel.valueDisabled.value).toBe(true)
    expect(validationButton().props.disabled).toBe(true)
    panel.editDraft({ target: { value: '{"bad":true}' } })
    await panel.chooseSource('inherit')
    await panel.validate()
    await panel.save()
    expect(writes).toEqual([])
  })

  test('the rendered validation button allows atomic leaf writes and requires confirmation before group resets', async () => {
    const group = 'permission.tools.rules."shell.*"'
    const bundle = sources()
    Object.assign(bundle.global, {
      has_override: true,
      override_path: group,
      requires_group_reset: false,
      default_value: 'allow',
      layer_value: null,
    })
    const { panel, writes, validationButton } = await advancedEditor(bundle)
    panel.settingPath.value = `${group}.routine`
    await panel.load()
    panel.editDraft({ target: { value: '"ask"' } })
    expect(validationButton().props.disabled).toBe(false)
    await validationButton().props.onClick()
    expect(writes[0]).toMatchObject({
      action: 'set',
      path: `${group}.routine`,
      value: 'ask',
      options: { dry_run: true },
    })
    panel.sourceMode.value = 'inherit'
    expect(validationButton().props.disabled).toBe(true)
    await validationButton().props.onClick()
    expect(writes).toHaveLength(1)
    panel.editDraft({ target: { value: '"ask"' } })
    await panel.chooseSource('inherit')
    expect(validationButton().props.disabled).toBe(false)
    await validationButton().props.onClick()
    expect(writes[1]).toMatchObject({ action: 'delete', path: group, options: { dry_run: true } })
    panel.activeAction.value = 'save'
    expect(validationButton().props.disabled).toBe(true)
  })

  test('the rendered validation button keeps opaque ancestors blocked until their reset is confirmed', async () => {
    const bundle = sources(null)
    Object.assign(bundle.global, {
      has_override: true,
      override_path: 'ui',
      requires_group_reset: true,
      default_value: null,
      layer_value: null,
    })
    const { panel, writes, validationButton } = await advancedEditor(bundle)
    expect(validationButton().props.disabled).toBe(true)
    await validationButton().props.onClick()
    expect(writes).toHaveLength(0)
    await panel.chooseSource('inherit')
    expect(validationButton().props.disabled).toBe(false)
    await validationButton().props.onClick()
    expect(writes[0]).toMatchObject({ action: 'delete', path: 'ui', options: { dry_run: true } })
  })

  test.each([
    ['workspace', 'providers.default_selection', 'thinking_mode', 'high'],
    ['global', 'permission.tools.rules."shell.*"', 'routine', 'allow'],
  ] as const)(
    'a missing child in %s restores its atomic group %s rather than deleting the absent leaf',
    async (scope, group, child, base) => {
      const bundle = sources()
      Object.assign(bundle[scope], {
        has_override: true,
        override_path: group,
        requires_group_reset: false,
        default_value: base,
        inherited_value: scope === 'workspace' ? base : undefined,
        layer_value: null,
      })
      const { panel, writes, confirmations, props } = await advancedEditor(bundle)
      props.scope = scope
      await vue.nextTick()
      panel.settingPath.value = `${group}.${child}`
      await panel.load()
      expect(panel.sourceMode.value).toBe('local')
      expect(panel.valueDisabled.value).toBe(false)
      expect(panel.draftText.value).toBe('null')
      const mode = scope === 'workspace' ? 'high' : 'ask'
      panel.editDraft({ target: { value: JSON.stringify(mode) } })
      expect(panel.dirty.value).toBe(true)
      await panel.save()
      expect(writes[0]).toMatchObject({ action: 'set', path: `${group}.${child}`, value: mode, layer: scope })
      expect(confirmations).toHaveLength(0)
      await panel.chooseSource('inherit')
      await panel.save()
      expect(confirmations[0]).toMatchObject({
        confirmText: scope === 'global' ? 'Reset to defaults' : 'Use inherited value',
      })
      expect(writes).toHaveLength(2)
      expect(writes[1]).toMatchObject({ action: 'delete', path: group, layer: scope })
    },
  )

  test('a delayed group-reset confirmation cannot change the newly selected layer', async () => {
    const bundle = sources(null, { value: false })
    Object.assign(bundle.global, { has_override: true, override_path: 'ui', default_value: {} })
    const { panel, writes, props, confirm } = await advancedEditor(bundle)
    let resolveConfirmation!: (accepted: boolean) => void
    confirm(
      new Promise<boolean>((resolve) => {
        resolveConfirmation = resolve
      }),
    )
    const pending = panel.chooseSource('inherit')
    props.scope = 'workspace'
    await vue.nextTick()
    resolveConfirmation(true)
    await pending
    expect(panel.sourceMode.value).toBe('local')
    expect(JSON.parse(panel.draftText.value)).toEqual({ value: false })
    expect(panel.dirty.value).toBe(false)
    await panel.save()
    expect(writes).toEqual([])
  })

  test('an ancestor JSON override restores its actual boundary only after group confirmation', async () => {
    const bundle = sources(null, undefined, 'dark')
    Object.assign(bundle.global, { has_override: true, override_path: 'ui', requires_group_reset: true })
    const { panel, writes, confirm, confirmations } = await advancedEditor(bundle)
    expect(panel.valueDisabled.value).toBe(true)
    panel.editDraft({ target: { value: '"light"' } })
    expect(panel.draftText.value).toBe('null')
    confirm(false)
    await panel.chooseSource('inherit')
    await panel.save()
    expect(writes).toHaveLength(0)
    confirm(true)
    await panel.chooseSource('inherit')
    await panel.validate()
    await panel.save()
    expect(confirmations).toHaveLength(2)
    expect(confirmations[1]).toMatchObject({
      title: 'Restore defaults for {path}?',
      confirmText: 'Reset to defaults',
      description: 'This also restores the default values of the other settings in this group.',
    })
    expect(writes.map(({ action, path }) => ({ action, path }))).toEqual([
      { action: 'delete', path: 'ui' },
      { action: 'delete', path: 'ui' },
    ])
    expect(panel.sourceMode.value).toBe('inherit')
    expect(panel.valueDisabled.value).toBe(false)
  })

  test('raw partial JSON stays sparse while metadata provides separate resolved/default values', async () => {
    const own = { nested: { count: 0 } }
    const defaults = { nested: { count: 10, enabled: true }, untouched: ['builtin'] }
    const resolved = { ...defaults, nested: { count: 0, enabled: true } }
    const bundle = sources(own, { untouched: ['workspace'] }, undefined, { nested: { count: 99 } })
    Object.assign(bundle.global, { default_value: defaults, layer_value: resolved, has_override: true })
    const { panel, writes, props } = await advancedEditor(bundle)
    expect(JSON.parse(panel.draftText.value)).toEqual(own)
    expect(panel.inheritedValue.value).toEqual(defaults)
    await panel.save()
    expect(writes).toEqual([])
    panel.editDraft({ target: { value: JSON.stringify({ nested: { count: 0, enabled: false } }) } })
    await panel.save()
    expect(writes[0]?.value).toEqual({ nested: { count: 0, enabled: false } })
    props.scope = 'workspace'
    await vue.nextTick()
    expect(JSON.parse(panel.draftText.value)).toEqual({ untouched: ['workspace'] })
  })

  test('raw null remains an explicit local JSON value when the server reports its presence', async () => {
    const bundle = sources(null, undefined, { theme: 'dark' })
    Object.assign(bundle.global, { has_override: true })
    const { panel } = await advancedEditor(bundle)
    expect(panel.sourceMode.value).toBe('local')
    expect(panel.draftText.value).toBe('null')
    expect(panel.dirty.value).toBe(false)
  })

  test('advanced source headings distinguish global defaults from workspace inheritance', async () => {
    const { panel, props } = await advancedEditor(sources(undefined, undefined, { enabled: true }))
    expect(panel.sourceValueHeading.value).toBe('Default value')
    props.scope = 'workspace'
    await vue.nextTick()
    expect(panel.sourceValueHeading.value).toBe('Inherited value')
  })

  test('a known null parent is not pinned as a synthetic local override', async () => {
    const bundle = sources()
    bundle.global.inherited_value = null
    const { panel, writes } = await advancedEditor(bundle)
    expect(panel.inheritedKnown.value).toBe(true)
    panel.chooseSource('local')
    expect(panel.sourceMode.value).toBe('inherit')
    expect(writes).toHaveLength(0)
  })

  test('missing global raw JSON stays empty and saves only directly entered fields', async () => {
    const value = { enabled: false, rules: ['default'] }
    const bundle = sources(undefined, undefined, undefined, { enabled: true, rules: ['runtime'] })
    Object.assign(bundle.global, { default_value: value, layer_value: value })
    const { panel, writes } = await advancedEditor(bundle)
    expect(JSON.parse(panel.draftText.value)).toBeNull()
    expect(panel.inheritedValue.value).toEqual(value)
    expect(panel.dirty.value).toBe(false)
    await panel.chooseSource('local')
    await panel.save()
    expect(writes).toEqual([])
    panel.editDraft({ target: { value: JSON.stringify({ enabled: true }) } })
    expect(panel.dirty.value).toBe(true)
    await panel.save()
    expect(writes[0]).toMatchObject({
      action: 'set',
      layer: 'global',
      path: 'ui.example',
      value: { enabled: true },
    })
  })

  test('only explicit lower-layer copying materializes a complete parent JSON group', async () => {
    const parent = { enabled: false, nested: { count: 0 }, rules: [] }
    const { panel, writes, props } = await advancedEditor(sources(parent, undefined, parent))
    props.scope = 'workspace'
    await vue.nextTick()
    expect(panel.draftText.value).toBe('null')
    await panel.chooseSource('local')
    expect(JSON.parse(panel.draftText.value)).toEqual(parent)
    await panel.save()
    expect(writes[0]).toMatchObject({ action: 'set', value: parent, layer: 'workspace' })
  })

  test('restore inheritance validates and saves deletion instead of writing the parent snapshot', async () => {
    const { panel, writes, state } = await advancedEditor(
      sources({ enabled: false }, { enabled: true }, { enabled: true }),
    )
    panel.chooseSource('inherit')
    expect(panel.dirty.value).toBe(true)
    await panel.validate()
    expect(writes[0]).toMatchObject({ action: 'delete', options: { dry_run: true, reload: false } })
    expect(state.global.value).toEqual({ enabled: false })
    await panel.save()
    expect(writes[1]).toMatchObject({ action: 'delete', layer: 'global' })
    expect(state.workspace.value).toEqual({ enabled: true })
    expect(panel.sourceMode.value).toBe('inherit')
    expect(panel.dirty.value).toBe(false)
  })

  test('an unavailable parent cannot be replaced with the unrelated final effective JSON', async () => {
    const { panel, writes } = await advancedEditor(sources(undefined, undefined, undefined, { secret: 'runtime' }))
    expect(panel.inheritedKnown.value).toBe(false)
    panel.chooseSource('local')
    expect(panel.dirty.value).toBe(false)
    expect(writes).toHaveLength(0)
  })

  test('editing an inherited false value directly creates an explicit local value', async () => {
    const { panel, writes } = await advancedEditor(sources(undefined, undefined, false, true))
    panel.editDraft({ target: { value: 'false' } })
    await panel.save()
    expect(writes[0]).toMatchObject({ action: 'set', value: false })
    expect(panel.sourceMode.value).toBe('local')
  })

  test('a failed restore retains the source draft for retry', async () => {
    const { panel, fail } = await advancedEditor(sources({ enabled: false }, undefined, { enabled: true }))
    panel.chooseSource('inherit')
    fail('write failed')
    await panel.save()
    expect(panel.error.value).toBe('write failed')
    expect(panel.dirty.value).toBe(true)
    expect(panel.sourceMode.value).toBe('inherit')
    fail('')
    await panel.save()
    expect(panel.dirty.value).toBe(false)
  })
})
