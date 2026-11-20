// Real SettingsPage, shared source controls, and native browser input.
// BOT_PROBE_OFFLINE=1 LAYERED_SETTINGS_PROBE_ONLY=1 node scripts/probe-bot.cjs
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const clone = (value) => JSON.parse(JSON.stringify(value ?? null))
const own = (object, key) => Object.prototype.hasOwnProperty.call(object, key)

function documentFromFlat(flat) {
  const document = {}
  for (const [key, value] of Object.entries(flat)) {
    const parts = key.split('.')
    let parent = document
    for (const part of parts.slice(0, -1)) parent = parent[part] ||= {}
    parent[parts.at(-1)] = clone(value)
  }
  return document
}
function flatValue(flat, key) {
  if (!key) return documentFromFlat(flat)
  if (key.startsWith('ui.tui.') && own(flat, 'ui.tui') && flat['ui.tui'] === null) return null
  if (own(flat, key)) return clone(flat[key])
  return key.split('.').reduce((value, part) => value?.[part], documentFromFlat(flat)) ?? null
}

function createFixture({ execution }) {
  const primary = { provider: 'fixture', adapter: 'local', model: 'fixture-model' }
  const alternate = { provider: 'fixture', adapter: 'local', model: 'alternate-model', parallel_tool_calls: false }
  const defaults = {
    'session.auto_title.enabled': true,
    'session.auto_title.message_interval': 5,
    'session.auto_title.cooldown_seconds': 300,
    'session.compaction.auto': true,
    'session.compaction.reserved_tokens': null,
    'runtime.providers.client_versions.codex': '1.0.0',
    'runtime.providers.client_versions.claude': '1.0.0',
    'runtime.providers.client_versions.gemini': '1.0.0',
    'runtime.providers.client_versions.auto_update': true,
    'ui.tui.theme': 'fixture-theme',
    'ui.tui.color_scheme': 'dark',
    'ui.tui.graphics': 'auto',
    'providers.default_selection': primary,
    'runtime.auxiliary_model': null,
    'permission.approval_model': null,
    sandbox: true,
    resource_guard: true,
  }
  const values = {
    global: {
      'session.auto_title.enabled': false,
      'session.auto_title.cooldown_seconds': 0,
      'runtime.providers.client_versions.codex': '',
      'providers.default_selection': primary,
      'ui.tui.theme': null,
      sandbox: false,
    },
    workspace: {
      'session.auto_title.enabled': true,
      'session.auto_title.cooldown_seconds': 60,
      'runtime.providers.client_versions.codex': '9.0.0',
      'providers.default_selection': alternate,
      sandbox: true,
    },
  }
  const selected = {}
  const unknownParents = new Set()
  const writes = []
  const validations = []
  const parent = (layer) => (layer === 'global' ? defaults : { ...defaults, ...values.global })
  const effective = () => ({ ...defaults, ...values.global, ...values.workspace, sandbox: false })
  const layerRead = (layer, key) => {
    const raw = values[layer]
    let requiresGroupReset = false
    let overridePath = flatValue(raw, key) !== null || (key.startsWith('ui.tui.') && own(raw, key)) ? key : undefined
    if (!overridePath && key.startsWith('ui.tui.') && own(raw, 'ui.tui') && raw['ui.tui'] === null) {
      overridePath = 'ui.tui'
      requiresGroupReset = true
    }
    if (layer === 'workspace') {
      for (const group of ['providers.default_selection', 'runtime.auxiliary_model', 'permission.approval_model']) {
        if (key.startsWith(group + '.') && own(raw, group) && raw[group] !== null) overridePath = group
      }
    }
    return {
      value: flatValue(raw, key),
      has_override: overridePath !== undefined,
      requires_group_reset: requiresGroupReset,
      ...(overridePath ? { override_path: overridePath } : {}),
      ...(unknownParents.has(layer + ':' + key)
        ? {}
        : {
            default_value: flatValue(defaults, key),
            layer_value: flatValue({ ...parent(layer), ...raw }, key),
            ...(layer === 'workspace' ? { inherited_value: flatValue(parent(layer), key) } : {}),
          }),
      config_path: '/fixture/' + layer + '/config.json',
      config_found: true,
      path: key || null,
    }
  }
  const bundle = (key) => ({
    effective: { value: flatValue(effective(), key), source: 'effective', path: key },
    file: layerRead('global', key),
    global: layerRead('global', key),
    workspace: layerRead('workspace', key),
  })
  const state = (id) => {
    const raw = selected[id] || {}
    const resolved = { ...alternate, ...raw }
    return {
      ...execution(id),
      execution: {
        ...execution(id).execution,
        selected_selection: clone(raw),
        inherited_selection: clone(alternate),
        model_provider_id: resolved.provider,
        model_adapter_id: resolved.adapter,
        model_id: resolved.model,
        model_thinking_mode: resolved.thinking_mode,
        model_speed_mode: resolved.speed_mode,
        model_verbosity: resolved.verbosity,
      },
    }
  }
  const body = async (req) => {
    let raw = ''
    for await (const chunk of req) raw += chunk
    return JSON.parse(raw || '{}')
  }
  return {
    values,
    writes,
    validations,
    selected,
    unknownParents,
    async handle(req, url, json) {
      const p = url.pathname
      const layer = p.match(/^\/api\/v1\/settings\/layers\/(global|workspace)$/)?.[1]
      if (p === '/api/v1/settings/sources') {
        json(Object.fromEntries(JSON.parse(url.searchParams.get('paths') || '[]').map((key) => [key, bundle(key)])))
      } else if (layer && req.method === 'GET') {
        json(layerRead(layer, url.searchParams.get('path') || ''))
      } else if (layer && req.method === 'PUT') {
        const update = await body(req)
        if (update.dry_run) {
          validations.push({ layer, ...update })
          json({ valid: true, changed: false })
          return true
        }
        const parts = update.path.split('.')
        const ancestor = parts.slice(0, -1).join('.')
        if (
          own(values[layer], ancestor) &&
          typeof values[layer][ancestor] === 'object' &&
          values[layer][ancestor] !== null
        )
          values[layer][ancestor][parts.at(-1)] = clone(update.value)
        else values[layer][update.path] = clone(update.value)
        writes.push({ method: 'PUT', layer, ...update })
        json({ changed: true, current: clone(update.value) })
      } else if (layer && req.method === 'DELETE') {
        const key = url.searchParams.get('path')
        for (const existing of Object.keys(values[layer]))
          if (existing === key || existing.startsWith(key + '.')) delete values[layer][existing]
        writes.push({ method: 'DELETE', layer, path: key })
        json({ changed: true, deleted: true })
      } else if (p === '/api/v1/settings' && req.method === 'GET') {
        const key = url.searchParams.get('path') || ''
        json(
          url.searchParams.get('source') === 'file' ? layerRead('global', key) : { value: flatValue(effective(), key) },
        )
      } else if (/^\/api\/v1\/sessions\/\d+\/state$/.test(p)) {
        json(state(Number(p.split('/')[4])))
      } else if (/^\/api\/v1\/sessions\/\d+\/selection$/.test(p) && req.method === 'PUT') {
        const id = Number(p.split('/')[4])
        const update = await body(req)
        selected[id] = {
          ...(update.model
            ? { provider: update.model.provider_id, adapter: update.model.adapter_id, model: update.model.model_id }
            : {}),
          ...Object.fromEntries(
            ['thinking_mode', 'speed_mode', 'verbosity'].filter((key) => update[key]).map((key) => [key, update[key]]),
          ),
          ...(typeof update.parallel_tool_calls === 'boolean'
            ? { parallel_tool_calls: update.parallel_tool_calls }
            : {}),
        }
        writes.push({ method: 'PUT', layer: 'session', sessionId: id, path: 'selection', value: clone(update) })
        json(state(id))
      } else return false
      return true
    },
  }
}

async function run({ cdp, base, evidence, wait, check, fixture }) {
  const field = (key) => '[data-server-setting=' + JSON.stringify(key) + ']'
  const sandbox = (key) => '[data-sandbox-field=' + JSON.stringify(key) + ']'
  const source = (selector, mode) => selector + ' [data-setting-source-choice=' + JSON.stringify(mode) + ']'
  const reset = (selector) => selector + ' [data-setting-reset]'
  const mode = (selector) =>
    cdp.evaluate(
      'document.querySelector(' + JSON.stringify(selector + ' [data-setting-source]') + ')?.dataset.settingSource',
    )
  const input = (selector) =>
    cdp.evaluate(
      '(() => {const e=document.querySelector(' +
        JSON.stringify(selector + ' input') +
        ');return e?.type==="checkbox"?e.checked:e?.value})()',
    )
  const text = (selector) => cdp.evaluate('document.querySelector(' + JSON.stringify(selector) + ')?.innerText || ""')
  const resetDisabled = (selector) =>
    cdp.evaluate('document.querySelector(' + JSON.stringify(reset(selector)) + ')?.disabled')
  const ready = (selector) =>
    wait(
      () =>
        cdp.evaluate(
          '(() => {const root=document.querySelector(' +
            JSON.stringify(selector) +
            ');if(!root||root.getAttribute("aria-busy")==="true")return false;' +
            'const source=root.querySelector("[data-setting-source-choice=inherit]");if(source)return !source.disabled;' +
            'const reset=root.querySelector("[data-setting-reset]");' +
            'return !!reset&&(!reset.disabled||[...root.querySelectorAll(\'input:not([type="hidden"]),textarea,button[aria-haspopup="dialog"]\')].some(input=>!input.disabled));})()',
        ),
      'loaded ' + selector,
    )
  const globalControls = async (selector, label) => {
    check(
      await cdp.evaluate(
        '(() => {const root=document.querySelector(' +
          JSON.stringify(selector) +
          ');const reset=root?.querySelector("[data-setting-reset]");return !!reset&&reset.getAttribute("aria-label").includes("Reset to defaults")' +
          '&&!root.querySelector("[data-setting-source],[data-setting-source-choice]");})()',
      ),
      label + ' offers direct editing and one reset action without a global source switch or badge',
    )
  }
  const navigate = (url) =>
    cdp.evaluate(
      '(() => {const t=new URL(' +
        JSON.stringify(url) +
        ',location.href);const id=new URLSearchParams(location.search).get("windowId");if(location.pathname.startsWith("/settings")&&id)t.searchParams.set("windowId",id);return window.__botProbe.router.push(t.pathname+t.search)})()',
    )
  const changeSource = async (selector, next, layer, key) => {
    await ready(selector)
    const count = fixture.writes.length
    await cdp.click(source(selector, next))
    await wait(() => fixture.writes.length > count, 'source choice writes ' + key)
    await ready(selector)
    await wait(async () => (await mode(selector)) === next, 'saved source ' + key)
    const write = fixture.writes.at(-1)
    check(
      write.layer === layer && write.path === key && write.method === (next === 'inherit' ? 'DELETE' : 'PUT'),
      layer + ' source switch persists only ' + key,
    )
    return write
  }
  const restoreDefault = async (selector, key) => {
    await ready(selector)
    const count = fixture.writes.length
    await cdp.click(reset(selector))
    await wait(() => fixture.writes.length > count, 'reset writes ' + key)
    await ready(selector)
    await wait(() => resetDisabled(selector), 'reset finishes ' + key)
    const write = fixture.writes.at(-1)
    check(
      write.layer === 'global' && write.path === key && write.method === 'DELETE',
      'global reset clears only ' + key,
    )
    return write
  }
  const screenshot = async (name) => {
    const { data } = await cdp.send('Page.captureScreenshot', { format: 'jpeg', quality: 80 })
    await fs.writeFile(path.join(evidence, name + '.jpg'), Buffer.from(data, 'base64'))
  }

  await cdp.send('Page.navigate', { url: base + '/chat?sessionId=7' })
  await wait(() => cdp.evaluate('window.__botProbe?.activeComposerSession()==="7"'), 'current project conversation')
  await cdp.evaluate('window.__botProbe.setAppLocale("en-US")')
  await navigate('/settings/runtime-session?view=compaction&scope=global')
  const enabled = field('session.auto_title.enabled')
  const cooldown = field('session.auto_title.cooldown_seconds')
  await wait(
    () => cdp.evaluate(`location.search.includes('view=session-behavior') && location.hash === '#settings-context-management'`),
    'compaction bookmark opens the context card in Session settings',
  )
  await ready(enabled)
  await ready(cooldown)
  await globalControls(enabled, 'Generic global boolean')
  await globalControls(cooldown, 'Generic global number')
  check(
    !(await resetDisabled(enabled)) && (await input(enabled)) === false,
    'global false remains editable when the workspace enables the setting',
  )
  check(
    (await text(enabled + ' [data-setting-effective]')).includes('Enabled'),
    'the later effective value is shown separately from the global editor',
  )
  check(
    !(await resetDisabled(cooldown)) && (await input(cooldown)) === '0',
    'explicit zero is displayed without a local-source badge or replacement by later values',
  )
  await screenshot('web-layered-settings-global')
  await restoreDefault(enabled, 'session.auto_title.enabled')
  check(
    (await input(enabled)) === true && !own(fixture.values.global, 'session.auto_title.enabled'),
    'restoring defaults removes the global key and fills the value from program defaults',
  )
  let count = fixture.writes.length
  await cdp.click(enabled + ' input[role="switch"]')
  await wait(() => fixture.writes.length > count, 'direct boolean edit saves')
  await ready(enabled)
  check(
    fixture.writes.at(-1).method === 'PUT' &&
      fixture.values.global['session.auto_title.enabled'] === false &&
      !(await resetDisabled(enabled)),
    'editing a default-filled global boolean directly saves false without first selecting a source',
  )
  await restoreDefault(cooldown, 'session.auto_title.cooldown_seconds')
  check((await input(cooldown)) === '300', 'global numeric defaults use 300 rather than the workspace value of 60')
  count = fixture.writes.length
  await cdp.fill(cooldown + ' input', '0')
  await cdp.key('Enter', 'Enter', 13)
  await wait(() => fixture.writes.length > count, 'numeric zero saves')
  await ready(cooldown)
  check(
    fixture.writes.at(-1).method === 'PUT' && fixture.values.global['session.auto_title.cooldown_seconds'] === 0,
    'direct numeric editing preserves an explicit zero without a source operation',
  )

  await navigate('/settings/runtime-session?view=client-versions&scope=global')
  const version = field('runtime.providers.client_versions.codex')
  await ready(version)
  await globalControls(version, 'Generic global text')
  check(
    !(await resetDisabled(version)) && (await input(version)) === '',
    'a saved empty string is displayed as the global value',
  )
  await restoreDefault(version, 'runtime.providers.client_versions.codex')
  check((await input(version)) === '1.0.0', 'resetting text uses the program default rather than the workspace value')
  count = fixture.writes.length
  await cdp.fill(version + ' input', '')
  await cdp.key('Enter', 'Enter', 13)
  await wait(() => fixture.writes.length > count, 'explicit empty text saves')
  await ready(version)
  check(
    fixture.writes.at(-1).method === 'PUT' &&
      fixture.values.global['runtime.providers.client_versions.codex'] === '' &&
      !(await resetDisabled(version)),
    'an explicit empty string is saved as a value and is not converted to reset',
  )

  delete fixture.values.global['session.auto_title.enabled']
  fixture.unknownParents.add('global:session.auto_title.enabled')
  await navigate('/settings/runtime-session?view=compaction&scope=global')
  await ready(enabled)
  await globalControls(enabled, 'Global setting with unavailable default metadata')
  check(
    await resetDisabled(enabled),
    'an absent global setting has nothing to reset when default metadata is unavailable',
  )
  check(
    (await text(enabled)).includes('Unavailable') &&
      (await cdp.evaluate('!document.querySelector(' + JSON.stringify(enabled + ' input[role="switch"]') + ')')),
    'an unknown boolean is shown as unavailable instead of an invented enabled or disabled value',
  )
  const explicitValue = false
  count = fixture.writes.length
  await cdp.click(enabled + ' button[aria-haspopup="dialog"]')
  await cdp.clickText('[data-option-menu-item] .font-medium', 'Disabled', true)
  await wait(() => fixture.writes.length > count, 'explicit edit without default metadata saves')
  await ready(enabled)
  check(
    fixture.values.global['session.auto_title.enabled'] === explicitValue && !(await resetDisabled(enabled)),
    'a deliberate value can be saved directly when default metadata is unavailable',
  )
  fixture.unknownParents.clear()

  await navigate('/settings/interface?view=tui&scope=global')
  const theme = field('ui.tui.theme')
  const graphics = field('ui.tui.graphics')
  await ready(theme)
  await globalControls(theme, 'Global UI preference')
  check(!(await resetDisabled(theme)), 'an explicitly cleared UI preference remains resettable')
  await restoreDefault(theme, 'ui.tui.theme')
  check(
    (await text(theme + ' button[aria-haspopup="dialog"]')).includes('fixture-theme'),
    'resetting a cleared UI preference fills the actual default theme',
  )
  await cdp.click(theme + ' button[aria-haspopup="dialog"]')
  check(
    !(await text('[data-keyboard-overlay="true"]')).includes('Default / unset'),
    'the global value menu has no reset or default-source option',
  )
  await cdp.key('Escape', 'Escape', 27)
  fixture.values.global['ui.tui'] = null
  await navigate('/settings/runtime-session?view=compaction&scope=global')
  await ready(enabled)
  await navigate('/settings/interface?view=tui&scope=global')
  await ready(theme)
  await ready(graphics)
  check(
    !(await resetDisabled(theme)) && !(await resetDisabled(graphics)),
    'an explicitly cleared UI group exposes reset actions for its affected descendants',
  )
  check(
    await cdp.evaluate(
      'document.querySelector(' + JSON.stringify(theme + ' button[aria-haspopup="dialog"]') + ').disabled',
    ),
    'a child editor cannot silently replace its cleared parent group',
  )
  await cdp.click(reset(theme))
  await wait(async () => (await text('[role="dialog"]')).includes('ui.tui'), 'group reset explains its actual path')
  count = fixture.writes.length
  await cdp.clickText('[role="dialog"] button', 'Cancel', true)
  check(
    fixture.writes.length === count && fixture.values.global['ui.tui'] === null,
    'cancelling restore defaults preserves the complete explicit clear',
  )
  await cdp.click(reset(theme))
  await wait(async () => (await text('[role="dialog"]')).includes('ui.tui'), 'group reset confirmation reopens')
  await cdp.clickText('[role="dialog"] button', 'Reset to defaults', true)
  await wait(() => fixture.writes.length > count, 'group default reset is saved')
  await ready(theme)
  await ready(graphics)
  await wait(
    async () => (await resetDisabled(theme)) && (await resetDisabled(graphics)),
    'siblings refresh after resetting the shared group',
  )
  check(
    fixture.writes.at(-1).method === 'DELETE' && fixture.writes.at(-1).path === 'ui.tui',
    'group reset removes the actual ancestor setting',
  )
  check(!own(fixture.values.global, 'ui.tui'), 'successful group reset refreshes sibling values and reset availability')

  await navigate('/settings/permissions?view=sandbox&scope=global')
  const isolation = sandbox('sandbox')
  await ready(isolation)
  await globalControls(isolation, 'Global sandbox')
  check(
    !(await resetDisabled(isolation)) && (await input(isolation)) === false,
    'sandbox global editor shows its own disabled value',
  )
  await restoreDefault(isolation, 'sandbox')
  check(
    (await input(isolation)) === true &&
      (await text('[data-sandbox-settings]')).includes('Direct execution') &&
      (await text('[data-sandbox-settings]')).includes('The runtime environment has disabled the sandbox.'),
    'sandbox defaults stay distinct from an environment-disabled execution state',
  )
  for (const value of [false, true]) {
    count = fixture.writes.length
    await cdp.click(isolation + ' input[role="switch"]')
    await wait(() => fixture.writes.length > count, 'direct sandbox edit saves')
    await ready(isolation)
    check(
      fixture.writes.at(-1).method === 'PUT' && fixture.values.global.sandbox === value,
      'sandbox can directly save ' + value + ' without a global source choice',
    )
  }
  check(!(await resetDisabled(isolation)), 'an explicitly saved value equal to the default can still be reset')
  await navigate('/settings/permissions?view=sandbox&scope=workspace')
  await ready(isolation)
  await changeSource(isolation, 'inherit', 'workspace', 'sandbox')
  check(
    (await cdp.evaluate(
      'document.querySelector(' + JSON.stringify(isolation + ' input[role="switch"]') + ').disabled',
    )) &&
      (await input(isolation)) === true &&
      (await text(isolation + ' [data-setting-inherited-source]')).includes('Global'),
    'inherited sandbox shows the global value and source with its switch disabled',
  )
  const sandboxPin = await changeSource(isolation, 'local', 'workspace', 'sandbox')
  check(
    await cdp.evaluate('!document.querySelector(' + JSON.stringify(isolation + ' input[role="switch"]') + ').disabled'),
    'explicitly selecting this scope enables sandbox editing',
  )
  check(
    sandboxPin.value === true && fixture.values.global.sandbox === true,
    'workspace keeps inheritance and explicit overrides without changing the global setting',
  )
  await screenshot('web-layered-settings-sandbox')

  const model = '[data-model-selection="default"]'
  const approval = '[data-model-selection="approval"]'
  const auxiliary = '[data-model-selection="auxiliary"]'
  const modelRoute = model + ' button[aria-haspopup="dialog"]'
  const saveModel = async (selector = model) => {
    const count = fixture.writes.length
    const saveButton = selector + ' [data-model-save]'
    await cdp.click(saveButton)
    await wait(() => fixture.writes.length > count, 'model selection is saved')
    await ready(selector)
    await wait(
      () => cdp.evaluate('!document.querySelector(' + JSON.stringify(saveButton) + ')'),
      'model editor is clean after save',
    )
    return fixture.writes.at(-1)
  }
  await navigate('/settings/models-providers?view=approval-model&scope=global')
  await wait(
    () => cdp.evaluate(`location.pathname === '/settings/runtime-session' && location.search.includes('view=session-behavior') && location.hash === '#settings-approval-model'`),
    'old approval bookmark opens the approval model in Session settings',
  )
  await ready(approval)
  await ready(auxiliary)
  check(
    Boolean(await text('#settings-context-management')) && Boolean(await text(enabled)),
    'the three model roles share the page with session titles and context management',
  )
  for (const [selector, path, label] of [
    [approval, 'permission.approval_model', 'Save approval model'],
    [auxiliary, 'runtime.auxiliary_model', 'Save auxiliary model'],
  ]) {
    await cdp.click(selector + ' button[aria-haspopup="dialog"]')
    await cdp.clickText('[data-option-menu-item] .font-medium', 'Alternate Model', true)
    const write = await saveModel(selector)
    check(write.layer === 'global' && write.path === path && write.value.model === 'alternate-model', label + ' writes only its model role')
  }
  await ready(model)
  await globalControls(model, 'Global model')
  check(
    !(await resetDisabled(model)) && (await text(modelRoute)).includes('Fixture Model'),
    'global model editor displays its own selection rather than the workspace model',
  )
  await cdp.click(reset(model))
  let write = await saveModel()
  check(
    write.method === 'DELETE' && write.layer === 'global' && write.path === 'providers.default_selection',
    'model reset clears only the global selection group',
  )
  await cdp.click(modelRoute)
  await cdp.clickText('[data-option-menu-item] .font-medium', 'Alternate Model', true)
  write = await saveModel()
  check(
    write.method === 'PUT' && write.layer === 'global' && write.value.model === 'alternate-model',
    'choosing a global model directly saves the selection without a source step',
  )
  await cdp.click(reset(model))
  await saveModel()
  check(
    (await resetDisabled(model)) && (await text(modelRoute)).includes('Fixture Model'),
    'resetting the edited global model fills its default and disables redundant reset',
  )
  await navigate('/settings/runtime-session?view=session-behavior&scope=workspace')
  await ready(enabled)
  check(
    await cdp.evaluate(`document.querySelector('.settings-workbench-location')?.title === '/fixture/workspace/config.json'`),
    'workspace Session settings shows the workspace file it edits',
  )
  const workspaceTitle = await changeSource(enabled, 'inherit', 'workspace', 'session.auto_title.enabled')
  check(
    workspaceTitle.method === 'DELETE' && fixture.values.global['session.auto_title.enabled'] === false,
    'restoring workspace title inheritance preserves the global setting',
  )
  await changeSource(enabled, 'local', 'workspace', 'session.auto_title.enabled')
  const compaction = field('session.compaction.auto')
  const workspaceCompaction = await changeSource(compaction, 'local', 'workspace', 'session.compaction.auto')
  check(
    workspaceCompaction.value === true && !own(fixture.values.global, 'session.compaction.auto'),
    'context management uses the same selected workspace layer',
  )
  await changeSource(compaction, 'inherit', 'workspace', 'session.compaction.auto')
  await navigate('/settings/models-providers?view=model-settings&scope=session')
  await ready(model)
  check(
    await cdp.evaluate(`location.pathname === '/settings/runtime-session' && location.search.includes('view=session-behavior') && !document.querySelector('${approval}, ${auxiliary}, #settings-context-management')`),
    'the old model settings link preserves Session scope and shows its own model override',
  )
  check(
    (await mode(model)) === 'inherit' && (await text(modelRoute)).includes('Alternate Model'),
    'session model still shows inheritance using its authoritative parent selection',
  )
  check(
    (await cdp.evaluate('document.querySelector(' + JSON.stringify(modelRoute) + ').disabled')) &&
      (await text(model + ' [data-setting-inherited-source]')).includes('Parent configuration'),
    'an inherited session model displays its parent and disables model selection',
  )
  await cdp.click(source(model, 'local'))
  check(
    await cdp.evaluate('!document.querySelector(' + JSON.stringify(modelRoute) + ').disabled'),
    'the model picker becomes editable after explicitly selecting this scope',
  )
  await cdp.evaluate(`(() => {
    const url = new URL(location.href);
    url.searchParams.set('targetSessionId', '8');
    void window.__botProbe.router.replace(url.pathname + url.search);
  })()`)
  await wait(() => cdp.evaluate(`document.querySelector('[role="dialog"]')?.textContent.includes('Discard unsaved model changes?')`), 'changing the conversation protects its model draft')
  await cdp.clickText('[role="dialog"] button', 'Keep editing', true)
  await wait(() => cdp.evaluate(`!document.querySelector('[role="dialog"]')`), 'cancelled conversation change')
  write = await saveModel()
  check(
    write.layer === 'session' &&
      write.sessionId === 7 &&
      write.value.model?.model_id === 'alternate-model' &&
      write.value.parallel_tool_calls === false,
    'session pin saves the inherited model in the current conversation',
  )
  await cdp.click(source(model, 'inherit'))
  write = await saveModel()
  check(
    write.layer === 'session' && Object.keys(write.value).length === 0,
    'session restore saves an empty selection and keeps the parent model',
  )
  check(
    (await mode(model)) === 'inherit' && (await text(modelRoute)).includes('Alternate Model'),
    'session remains inherited after the save reload',
  )
  await screenshot('web-layered-settings-models')

  await navigate('/settings/diagnostics?view=advanced-settings&scope=global')
  const advanced = '[data-advanced-setting]'
  await ready(advanced)
  await cdp.click('button[aria-haspopup="dialog"][aria-label^="Configuration path"]')
  await cdp.clickText('[data-option-menu-item] .font-medium', 'session.compaction', true)
  await wait(
    () =>
      cdp.evaluate(
        'document.querySelector("label[for=advanced-setting-draft]")?.textContent.trim()==="session.compaction"',
      ),
    'advanced editor loads the requested group',
  )
  await ready(advanced)
  await globalControls(advanced, 'Global advanced JSON')
  check(await resetDisabled(advanced), 'an absent advanced global group has nothing to reset')
  const custom = { auto: false, reserved_tokens: 0 }
  await cdp.fill('#advanced-setting-draft', JSON.stringify(custom, null, 2))
  count = fixture.writes.length
  await cdp.clickText('button', 'Save & reload', true)
  await wait(() => fixture.writes.length > count, 'advanced direct edit saves')
  await ready(advanced)
  check(
    fixture.writes.at(-1).method === 'PUT' &&
      fixture.writes.at(-1).layer === 'global' &&
      fixture.writes.at(-1).path === 'session.compaction' &&
      JSON.stringify(fixture.writes.at(-1).value) === JSON.stringify(custom),
    'typing global JSON directly saves its exact false and zero values',
  )
  await cdp.click(reset(advanced))
  check(
    (await text(advanced)).includes('Save to restore default settings.'),
    'advanced default reset stays a clearly named draft until saved',
  )
  count = fixture.writes.length
  await cdp.clickText('button', 'Save & reload', true)
  await wait(() => fixture.writes.length > count, 'advanced reset saves')
  await ready(advanced)
  check(
    fixture.writes.at(-1).method === 'DELETE' &&
      fixture.writes.at(-1).path === 'session.compaction' &&
      !own(fixture.values.global, 'session.compaction') &&
      (await resetDisabled(advanced)),
    'advanced reset clears only its selected group and returns to default-filled values',
  )
  await screenshot('web-layered-settings-advanced')

  await navigate('/settings/diagnostics?view=advanced-settings&scope=workspace')
  await ready(advanced)
  check(
    (await mode(advanced)) === 'inherit' &&
      (await cdp.evaluate(
        '(() => {const field=document.querySelector("#advanced-setting-draft"); return field.disabled && JSON.parse(field.value).auto===true;})()',
      )) &&
      (await text(advanced + ' [data-setting-inherited-source]')).includes('Global'),
    'inherited advanced JSON shows the global document in a disabled editor',
  )
  count = fixture.writes.length
  await cdp.click(source(advanced, 'local'))
  check(
    fixture.writes.length === count &&
      (await cdp.evaluate('!document.querySelector("#advanced-setting-draft").disabled')),
    'choosing local advanced JSON enables an unsaved draft without writing it',
  )
  await cdp.fill('#advanced-setting-draft', JSON.stringify(custom, null, 2))
  await cdp.clickText('button', 'Save & reload', true)
  await wait(() => fixture.writes.length > count, 'local workspace JSON saves')
  await ready(advanced)
  check(
    fixture.writes.at(-1).layer === 'workspace' &&
      JSON.stringify(fixture.values.workspace['session.compaction']) === JSON.stringify(custom),
    'the explicitly local advanced document saves only to the workspace',
  )
  count = fixture.writes.length
  await cdp.click(source(advanced, 'inherit'))
  check(
    fixture.writes.length === count &&
      (await cdp.evaluate(
        '(() => {const field=document.querySelector("#advanced-setting-draft"); return field.disabled && JSON.parse(field.value).auto===true;})()',
      )),
    'restoring inheritance immediately displays the grey parent document before saving',
  )
  await cdp.clickText('button', 'Save & reload', true)
  await wait(() => fixture.writes.length > count, 'workspace JSON inheritance saves')
  await ready(advanced)
  check(
    fixture.writes.at(-1).method === 'DELETE' && !own(fixture.values.workspace, 'session.compaction'),
    'saving inherited advanced JSON deletes the override instead of writing its displayed parent document',
  )

  const modelGroup = 'providers.default_selection'
  const modelLeaf = modelGroup + '.thinking_mode'
  fixture.values.global[modelGroup] = {
    provider: 'fixture',
    adapter: 'local',
    model: 'fixture-model',
    thinking_mode: 'low',
  }
  fixture.values.workspace[modelGroup] = {
    provider: 'fixture',
    adapter: 'local',
    model: 'alternate-model',
    parallel_tool_calls: false,
  }
  await navigate('/settings/diagnostics?view=advanced-settings&scope=workspace')
  await ready(advanced)
  await cdp.click('button[aria-haspopup="dialog"][aria-label^="Configuration path"]')
  await cdp.fill('input[placeholder="Search common paths or type a custom path"]', modelLeaf)
  await cdp.clickText('[data-option-menu-item]', modelLeaf)
  await wait(
    () =>
      cdp.evaluate(
        'document.querySelector("label[for=advanced-setting-draft]")?.textContent.trim()===' +
          JSON.stringify(modelLeaf),
      ),
    'custom model leaf loads',
  )
  await ready(advanced)
  check(
    (await mode(advanced)) === 'local' &&
      (await cdp.evaluate(
        'document.querySelector("#advanced-setting-draft").value === "null" && !document.querySelector("#advanced-setting-draft").disabled',
      )),
    'a missing child of an explicit model group is local, stays raw null, and can be edited directly',
  )
  count = fixture.writes.length
  await cdp.fill('#advanced-setting-draft', JSON.stringify('high'))
  const validationCount = fixture.validations.length
  await cdp.clickText('button', 'Dry-run validate', true)
  await wait(() => fixture.validations.length > validationCount, 'atomic model child can be validated')
  await ready(advanced)
  check(
    fixture.validations.at(-1).path === modelLeaf &&
      fixture.validations.at(-1).value === 'high' &&
      fixture.writes.length === count &&
      !own(fixture.values.workspace[modelGroup], 'thinking_mode'),
    'dry-run validation of an atomic group child uses the leaf path and does not save the draft',
  )
  await cdp.clickText('button', 'Save & reload', true)
  await wait(() => fixture.writes.length > count, 'model child saves')
  await ready(advanced)
  check(
    fixture.writes.at(-1).path === modelLeaf &&
      fixture.writes.at(-1).value === 'high' &&
      fixture.values.workspace[modelGroup].model === 'alternate-model' &&
      fixture.values.workspace[modelGroup].parallel_tool_calls === false &&
      fixture.values.workspace[modelGroup].thinking_mode === 'high',
    'editing an atomic group child writes only the leaf and preserves the model and parallel false',
  )
  count = fixture.writes.length
  await cdp.click(source(advanced, 'inherit'))
  await wait(() => cdp.evaluate('!!document.querySelector("[role=dialog]")'), 'whole model restore confirmation')
  check(
    (await text('[role="dialog"]')).includes(modelGroup),
    'restoring a model child identifies the entire atomic model group',
  )
  await cdp.clickText('[role="dialog"] button', 'Cancel', true)
  check(
    fixture.writes.length === count && (await mode(advanced)) === 'local',
    'cancelling a whole-model restore preserves the local group',
  )
  await cdp.click(source(advanced, 'inherit'))
  await cdp.clickText('[role="dialog"] button', 'Use inherited value', true)
  check(fixture.writes.length === count, 'confirmed model inheritance stays a draft until explicitly saved')
  await cdp.clickText('button', 'Save & reload', true)
  await wait(() => fixture.writes.length > count, 'whole model restore saves')
  await ready(advanced)
  check(
    fixture.writes.at(-1).method === 'DELETE' &&
      fixture.writes.at(-1).path === modelGroup &&
      !own(fixture.values.workspace, modelGroup) &&
      fixture.values.global[modelGroup].thinking_mode === 'low',
    'restoring model inheritance deletes the complete workspace group and preserves its global parent',
  )
  check(
    await cdp.evaluate('window.__botProbe.chat.selectedSessionId==="7" && window.__botProbe.lastConversation()==="7"'),
    'all setting edits preserve the active conversation',
  )
  assert(fixture.writes.length > 0)
}

module.exports = { createFixture, run }
