// Real SettingsPage, single permission editor and native pointer/keyboard input.
// BOT_PROBE_OFFLINE=1 PERMISSIONS_PROBE_ONLY=1 BOT_EVIDENCE=/tmp/agena-permissions node scripts/probe-bot.cjs
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')

const clone = (value) => JSON.parse(JSON.stringify(value))
const workspaceOnly = process.env.PERMISSIONS_WORKSPACE_ONLY === '1'
const permissionDefaults = {
  path: { workspace: 'readwrite', external: 'deny', rules: { '<state>': 'readwrite' } },
  network: { version: 2, default: 'allow', inbound: 'deny', hostLoopback: 'deny', allow: [], deny: [] },
  tools: {
    default: 'allow',
    names: { 'agena.interaction.ask': 'allow', 'agena.web.search': 'allow', 'agena.web.fetch': 'allow' },
    rules: {
      '*': { 'read-only': 'allow' },
      'shell.*': { 'no-op': 'allow', routine: 'allow', dangerous: 'deny' },
    },
  },
}
function merge(...policies) {
  return policies.reduce((result, policy) => {
    for (const [key, value] of Object.entries(policy || {})) {
      if (key === 'path' || key === 'tools') {
        const previous = result[key] || {}
        result[key] = { ...previous, ...clone(value) }
        for (const map of key === 'path' ? ['rules'] : ['names', 'rules']) {
          if (previous[map] || value[map]) result[key][map] = { ...previous[map], ...clone(value[map] || {}) }
        }
      } else if (key === 'network') {
        const network =
          value.version === 2
            ? value
            : { default: 'deny', inbound: 'deny', hostLoopback: 'deny', allow: [], deny: [], ...value }
        result.network = { ...result.network, ...clone(network), version: 2 }
      } else result[key] = clone(value)
    }
    return result
  }, {})
}
function createFixture({ execution }) {
  const defaults = workspaceOnly ? { ...permissionDefaults, tools: { default: 'allow' } } : permissionDefaults
  const values = {
    global: {
      path: { rules: { '<workspace>/docs': 'readonly' } },
      network: {
        version: 2,
        default: 'deny',
        inbound: 'deny',
        hostLoopback: 'deny',
        allow: [{ to: '198.51.100.24', protocol: 'tcp', ports: [443] }],
        deny: [{ to: '192.0.2.13' }],
      },
      tools: {
        names: { 'agena.fs.read': 'allow', 'agena.web.fetch': 'ask' },
      },
    },
    workspace: {},
    session: {},
  }
  const writes = [],
    ruleWrites = [],
    catalogRequests = [],
    workspaceCatalogRequests = [],
    sessionReads = [],
    rules = [
      { id: 700, scope: 'workspace', workspace_id: 7, tool_name: 'workspace.seven.fixture' },
      { id: 800, scope: 'workspace', workspace_id: 8, tool_name: 'workspace.eight.fixture' },
      { id: 801, scope: 'session', session_id: 8, tool_name: 'session.eight.fixture' },
    ].map((rule) => ({
      subject_kind: 'tool',
      action_key: rule.tool_name,
      mode: 'allow',
      source: 'user',
      created_at: '2026-01-01',
      updated_at: '2026-01-01',
      ...rule,
    }))
  const sessionValues = { 8: { path: { workspace: 'deny' }, tools: { default: 'ask' } } }
  if (workspaceOnly) values.global = { tools: { default: 'deny' } }
  const workspaceValues = { 8: { tools: { default: 'ask' } } }
  const workspaces = [
    { id: 7, path: '/fixture/project' },
    { id: 8, path: '/fixture/other-project' },
  ]
  const projectIds = [7, 8, ...Array.from({ length: 65 }, (_, index) => 100 + index)]
  const metadata = (id) => ({
    ...execution(id).session,
    ...(id === 8
      ? { title: 'Other project conversation', workspace_id: 8 }
      : id >= 100 && id <= 164
        ? { title: id === 164 ? 'Older project conversation' : 'Project conversation ' + id, workspace_id: 7 }
        : {}),
  })
  const selectedPermission = (id) => (id === 7 ? values.session : sessionValues[id] || {})
  const sessionParent = (id) => merge(defaults, values.global, metadata(id).workspace_id === 7 ? values.workspace : {})
  const inherited = (scope) =>
    scope === 'global'
      ? clone(defaults)
      : merge(defaults, values.global, ...(scope === 'session' ? [values.workspace] : []))
  const state = (id) => ({
    ...execution(id),
    session: metadata(id),
    execution: {
      ...execution(id).execution,
      selected_permission: clone(selectedPermission(id)),
      inherited_permission: sessionParent(id),
      effective_permission: merge(sessionParent(id), selectedPermission(id)),
    },
  })
  const body = async (req) => {
    let raw = ''
    for await (const chunk of req) raw += chunk
    return raw ? JSON.parse(raw) : {}
  }
  return {
    values,
    writes,
    ruleWrites,
    rules,
    sessionValues,
    workspaceValues,
    catalogRequests,
    workspaceCatalogRequests,
    sessionReads,
    async handle(req, url, json) {
      const p = url.pathname
      const layer = p.match(/^\/api\/v1\/settings\/layers\/(global|workspace)$/)?.[1]
      const workspaceId = layer === 'workspace' ? Number(url.searchParams.get('workspace_id') || 7) : null
      const document = workspaceId === 8 ? workspaceValues : values
      const documentKey = workspaceId === 8 ? 8 : layer
      const revision = () => JSON.stringify(document[documentKey])
      if (layer && url.searchParams.get('path') === 'permission_profile' && req.method === 'GET') {
        json({ value: null, revision: revision(), workspace_id: workspaceId })
      } else if (layer && url.searchParams.get('path') === 'permission' && req.method === 'GET') {
        json({
          value: clone(document[documentKey]),
          revision: revision(),
          default_value: clone(defaults),
          layer_value: merge(inherited(layer), document[documentKey]),
          ...(layer === 'workspace' ? {
            inherited_value: inherited(layer),
            workspace_id: workspaceId,
            active_workspace: workspaceId === 7,
          } : {}),
          config_path: workspaceId ? workspaces.find((workspace) => workspace.id === workspaceId).path + '/.agena/agena.json' : '/fixture/global/config.json',
          config_found: true,
        })
      } else if (layer && req.method === 'PUT') {
        const update = await body(req)
        assert.equal(update.path, 'permission')
        if (workspaceId !== null) assert.equal(update.expected_revision, revision())
        document[documentKey] = clone(update.value)
        writes.push({ scope: layer, workspaceId, ...update })
        json({ ok: true, current: clone(update.value), after_revision: revision(), reloaded: workspaceId !== 8 })
      } else if (layer && req.method === 'DELETE') {
        assert.equal(url.searchParams.get('path'), 'permission')
        if (workspaceId !== null) assert.equal(url.searchParams.get('expected_revision'), revision())
        document[documentKey] = {}
        writes.push({ scope: layer, workspaceId, path: 'permission', value: {} })
        json({ ok: true, current: null, after_revision: revision(), reloaded: workspaceId !== 8 })
      } else if (p === '/api/v1/settings' && url.searchParams.get('path') === 'permission') {
        json({ value: merge(defaults, values.global, values.workspace) })
      } else if (/^\/api\/v1\/sessions\/\d+\/state$/.test(p)) {
        const id = Number(p.split('/')[4])
        sessionReads.push(id)
        json(state(id))
      } else if (/^\/api\/v1\/sessions\/\d+\/permission$/.test(p) && req.method === 'PUT') {
        const update = await body(req)
        const id = Number(p.split('/')[4])
        if (id === 7) values.session = clone(update.permission)
        else sessionValues[id] = clone(update.permission)
        writes.push({
          scope: 'session',
          sessionId: id,
          path: 'permission',
          value: clone(update.permission),
        })
        json(state(Number(p.split('/')[4])))
      } else if (p === '/api/v1/permission-rules' && req.method === 'GET') {
        const id = Number(url.searchParams.get('workspace_id'))
        const items = id ? rules.filter((rule) => rule.scope === 'workspace' && rule.workspace_id === id) : rules
        json({ items: clone(items), page: { has_more: false } })
      } else if (p === '/api/v1/permission-rules' && req.method === 'POST') {
        const update = await body(req)
        ruleWrites.push(update)
        const rule = {
          id: Math.max(0, ...rules.map((item) => item.id)) + 1,
          action_key: update.tool_name,
          source: 'user',
          created_at: '2026-01-01',
          updated_at: '2026-01-01',
          workspace_id: update.scope === 'workspace' ? 7 : null,
          ...update,
        }
        rules.push(rule)
        json(rule)
      } else if (p === '/api/v1/permission-rules/batch' && req.method === 'POST') {
        const update = await body(req)
        ruleWrites.push(update)
        const affected = rules.filter((rule) => update.rule_ids.includes(rule.id) && rule.scope === update.scope &&
          (update.scope !== 'workspace' || rule.workspace_id === update.workspace_id))
        for (const rule of affected) {
          if (update.operation.action === 'delete') rules.splice(rules.indexOf(rule), 1)
          else rule.mode = update.operation.mode
        }
        json({ affected, missing_ids: update.rule_ids.filter((id) => !affected.some((rule) => rule.id === id)) })
      } else if (p === '/api/v1/sessions' && !url.searchParams.has('owner_bot_id')) {
        const query = (url.searchParams.get('search') || '').trim().toLowerCase()
        const limit = Math.max(1, Number(url.searchParams.get('limit')) || 30)
        const offset = Math.max(0, Number(url.searchParams.get('offset') || url.searchParams.get('cursor')) || 0)
        catalogRequests.push({ query, offset, limit })
        const matches = projectIds
          .map(metadata)
          .filter((session) => !query || (session.id + ' ' + session.title).toLowerCase().includes(query))
        const items = matches.slice(offset, offset + limit)
        const next = offset + items.length
        json({
          items,
          total: matches.length,
          page: { has_more: next < matches.length, next_cursor: next < matches.length ? String(next) : null },
        })
      } else if (/^\/api\/v1\/sessions\/\d+$/.test(p) && req.method === 'GET') {
        json(metadata(Number(p.split('/')[4])))
      } else if (p === '/api/v1/workspaces') {
        const query = (url.searchParams.get('search') || '').trim().toLowerCase()
        workspaceCatalogRequests.push(query)
        json({ items: workspaces.filter(({ path }) => path.toLowerCase().includes(query)), page: { has_more: false } })
      } else if (p === '/api/v1/workspaces/8') {
        json({ id: 8, path: '/fixture/other-project' })
      } else return false
      return true
    },
  }
}

async function run({ cdp, base, evidence, wait, check, visible, permissionFixture: fixture }) {
  const section = (id) => `[data-permission-section="${id}"]`
  const ruleGroup = (kind) => `[data-permission-rule-table="${kind}"]`
  const groupCounts = () =>
    cdp.evaluate(
      `Object.fromEntries([...document.querySelectorAll('[data-permission-rule-table]')].map(group=>[group.dataset.permissionRuleTable,group.querySelectorAll('[data-permission-rule]').length]))`,
    )
  const rule = (kind, target, condition) =>
    `[data-permission-rule=${JSON.stringify(kind)}][data-rule-target=${JSON.stringify(target)}]` +
    (condition === undefined ? '' : `[data-rule-condition=${JSON.stringify(condition)}]`)
  const frame = () =>
    cdp.evaluate('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))')
  const navigation = (url) => `(() => {
    const target = new URL(${JSON.stringify(url)}, location.href);
    const windowId = new URLSearchParams(location.search).get('windowId');
    if(location.pathname.startsWith('/settings') && windowId) target.searchParams.set('windowId', windowId);
    return window.__botProbe.router.push(target.pathname + target.search + target.hash);
  })()`
  const navigate = (url) => cdp.evaluate(navigation(url))
  const screenshot = async (name) => {
    const { data } = await cdp.send('Page.captureScreenshot', { format: 'jpeg', quality: 80 })
    await fs.writeFile(path.join(evidence, name + '.jpg'), Buffer.from(data, 'base64'))
  }
  const loaded = () =>
    cdp.evaluate(
      `document.querySelectorAll('[data-permission-section]').length===4 && document.querySelector('.permission-studio')?.getAttribute('aria-busy')==='false'`,
    )
  const sessionTrigger = '[data-permission-session-picker] button[aria-haspopup="dialog"]'
  const workspaceTrigger = '[data-permission-workspace-picker] button[aria-haspopup="dialog"]'
  const pickerSearch = '[data-keyboard-overlay="true"] input'
  const chooseWorkspace = async (label, search) => {
    await cdp.click(workspaceTrigger)
    await wait(() => visible(pickerSearch), 'workspace dropdown opens with search')
    await frame()
    await cdp.fill(pickerSearch, search)
    await wait(
      () => cdp.evaluate(`[...document.querySelectorAll('[data-option-menu-item] .font-medium')].some(item => item.textContent.trim() === ${JSON.stringify(label)})`),
      'workspace matches its catalog search',
    )
    await frame()
    await cdp.clickText('[data-option-menu-item] .font-medium', label, true)
  }
  const settleSessionPicker = async () => {
    await wait(
      () =>
        cdp.evaluate(`(() => {
          const retry = document.querySelector('[data-keyboard-overlay="true"] button[aria-label="Retry"], [data-keyboard-overlay="true"] button[aria-label="重试"]');
          return retry && !retry.disabled;
        })()`),
      'session catalog finishes loading before native pointer targeting',
    )
    // The shared mobile sheet centers again when the catalog changes its height.
    await frame()
  }
  const openSessionPicker = async () => {
    await cdp.click(sessionTrigger)
    await wait(() => visible(pickerSearch), 'shared session dropdown opens with search')
    await settleSessionPicker()
  }
  const chooseSession = async (label, hasDraft = false) => {
    await openSessionPicker()
    await cdp.fill(pickerSearch, label)
    await wait(
      () =>
        cdp.evaluate(
          `[...document.querySelectorAll('[data-option-menu-item] .font-medium')].some(item=>item.innerText.trim()===${JSON.stringify(label)})`,
        ),
      'requested conversation appears in the shared picker',
    )
    await settleSessionPicker()
    if (hasDraft)
      check(
        await visible('[data-permission-savebar]'),
        'opening and searching the session catalog preserves the pending permission draft',
      )
    await cdp.clickText('[data-option-menu-item] .font-medium', label, true)
  }
  const waitSession = async (id) => {
    await wait(
      async () =>
        fixture.sessionReads.includes(Number(id)) &&
        (await loaded()) &&
        (await cdp.evaluate(
          `new URLSearchParams(location.search).get('targetSessionId')===${JSON.stringify(String(id))}`,
        )),
      'permission editor loads the chosen settings session',
    )
    await frame()
  }
  const select = async (selector, label) => {
    await cdp.click(selector)
    await cdp.clickText('[data-option-menu-item] .font-medium', label, true)
    await frame()
  }
  const defaultTrigger = (key) => `[data-permission-default="${key}"] button[aria-haspopup="dialog"]`
  const sourceChoice = (key, source) => `[data-permission-default="${key}"] [data-setting-source-choice="${source}"]`
  const networkSourcesMenu = '[data-permission-network-source-menu]'
  const withNetworkSources = async (run) => {
    await cdp.click('[data-permission-network-sources]')
    await wait(() => visible(networkSourcesMenu), 'network rule sources open from the table header')
    await frame()
    const result = await run()
    await cdp.key('Escape', 'Escape', 27)
    await wait(async () => !(await visible(networkSourcesMenu)), 'network rule sources close with Escape')
    return result
  }
  const setNetworkListSource = (key, source) => withNetworkSources(() => cdp.click(sourceChoice(key, source)))
  const makeLocal = async (key) => {
    const inherited = await cdp.evaluate(
      'document.querySelector(' +
        JSON.stringify(`[data-permission-default="${key}"] [data-setting-source]`) +
        ')?.dataset.settingSource === "inherit"',
    )
    if (inherited) await cdp.click(sourceChoice(key, 'local'))
  }
  const inheritedControlsLocked = (keys) =>
    cdp.evaluate(`(${JSON.stringify(keys)}).every(key => {
    const row = document.querySelector('[data-permission-default="'+key+'"]');
    const editor = row?.querySelector('button[aria-haspopup="dialog"]');
    return !!editor && editor.disabled && !row.querySelector('[data-setting-source-choice="local"]').disabled;
  })`)
  const defaultSourcesMatch = (
    source,
    keys = ['path.workspace', 'path.external', 'tools.default', 'tools.read-only'],
  ) =>
    cdp.evaluate(`${JSON.stringify(keys)}.every(key => {
      const control = document.querySelector('[data-permission-default="'+key+'"] [data-setting-source]');
      return control?.dataset.settingSource===${JSON.stringify(source)} && control.querySelector('[data-setting-source-choice="'+${JSON.stringify(source)}+'"]')?.getAttribute('aria-pressed')==='true';
    })`)
  const compactSourcesMatch = (inheritLabel, localLabel, parentLabel) =>
    cdp.evaluate(`(() => {
      const controls = [...document.querySelectorAll('.permission-studio [data-setting-source]')]
        .filter(control => !control.closest('[data-sandbox-settings]'));
      return controls.length >= 8 && controls.every(control => {
        const inherit = control.querySelector('[data-setting-source-choice="inherit"]');
        const local = control.querySelector('[data-setting-source-choice="local"]');
        return inherit?.textContent.trim() === ${JSON.stringify(inheritLabel)} &&
          local?.textContent.trim() === ${JSON.stringify(localLabel)} &&
          inherit.title.includes(${JSON.stringify(parentLabel)}) &&
          !control.querySelector('[data-setting-inherited-source]');
      });
    })()`)
  const defaultChoicesMatch = (expected) =>
    cdp.evaluate(`Object.entries(${JSON.stringify(expected)}).every(([key,label]) => {
      const button = document.querySelector('[data-permission-default="'+key+'"] button[aria-haspopup="dialog"]');
      return button?.textContent.trim()===label && button.getAttribute('aria-label')?.endsWith(': '+label);
    })`)
  const save = async (label = 'Save permission') => {
    await cdp.clickText('[data-permission-savebar] button', label)
    await wait(async () => !(await visible('[data-permission-savebar]')), 'combined permission save completes')
  }
  const noOverflow = () =>
    cdp.evaluate(
      `(() => {
        const main=document.querySelector('.settings-page main');
        return document.documentElement.scrollWidth<=innerWidth+1 && main.scrollWidth<=main.clientWidth+1 &&
          [...main.querySelectorAll('.permission-studio,[data-permission-section],[data-permission-savebar],[data-saved-rules],[data-permission-advanced]')]
            .every(el=>el.getBoundingClientRect().right<=innerWidth+1);
      })()`,
    )
  const bulkFits = (selector, stacked) =>
    cdp.evaluate(`(() => {
      const bar = document.querySelector(${JSON.stringify(selector)});
      const selection = bar?.querySelector('.permission-bulk-selection')?.getBoundingClientRect();
      const controls = bar?.querySelector('.permission-bulk-controls')?.getBoundingClientRect();
      return !!selection && !!controls && bar.querySelectorAll('[role="status"]').length === 1 &&
        bar.scrollWidth <= bar.clientWidth + 1 &&
        (${stacked} ? controls.top >= selection.bottom : Math.abs(selection.top - controls.top) <= 1);
    })()`)
  const densityMeasurements = []
  const pageTop = async () => {
    await cdp.evaluate(
      `(() => { for(let element=document.querySelector('.permission-studio');element;element=element.parentElement) { if(element.scrollTop) element.scrollTo({top:0,behavior:'instant'}); } document.scrollingElement?.scrollTo({top:0,behavior:'instant'}); })()`,
    )
    await frame()
  }
  const captureDensity = async (name, mobile = false, theme = 'dark') => {
    await cdp.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: theme }] })
    await cdp.evaluate(
      `window.__botProbe.applyAppearanceSettingsToDom({useSystemTheme:false,themeVariant:${JSON.stringify(theme)}})`,
    )
    await cdp.evaluate('window.__botProbe.dismissToasts()')
    await wait(
      () => cdp.evaluate("document.querySelectorAll('[data-sonner-toast]').length===0"),
      'transient notices are dismissed before measuring the layout',
    )
    await pageTop()
    const measurement = await cdp.evaluate(`(() => {
      const describe = element => {
        const rect=element.getBoundingClientRect(), style=getComputedStyle(element);
        return {key:element.dataset.permissionDefault||element.dataset.permissionRule||'',text:element.innerText.trim().replace(/\\s+/g,' '),x:Math.round(rect.x),y:Math.round(rect.y),width:Math.round(rect.width),height:Math.round(rect.height),bottom:Math.round(rect.bottom),fontSize:style.fontSize,visible:rect.width>0&&rect.height>0&&rect.top>=0&&rect.bottom<=innerHeight-48};
      };
      const domains=[...document.querySelectorAll('[data-permission-section]')].map(element=>({id:element.dataset.permissionSection,...describe(element),defaults:[...element.querySelectorAll('.permission-domain-defaults [data-permission-default]')].map(describe)}));
      const ruleGroups=[...document.querySelectorAll('[data-permission-rule-table]')].map(group=>({
        kind:group.dataset.permissionRuleTable,
        domain:group.closest('[data-permission-section]')?.dataset.permissionSection,
        heading:group.querySelector('[data-permission-rule-heading]')?.innerText.trim(),
        ...describe(group),
        rows:[...group.querySelectorAll('[data-permission-rule]')].map(row=>({kind:row.dataset.permissionRule,...describe(row)}))
      }));
      const clippedModes=[...document.querySelectorAll('[data-permission-default] button[aria-haspopup="dialog"] .truncate,[data-permission-mode] .truncate')].filter(element=>element.getBoundingClientRect().height>0&&element.scrollWidth>element.clientWidth+1).map(element=>element.innerText);
      return {ruleLayout:describe(document.querySelector('[data-permission-rule-layout]')),viewport:{width:innerWidth,height:innerHeight},dark:document.documentElement.classList.contains('dark'),toolbar:document.querySelector('[data-permission-toolbar]')?describe(document.querySelector('[data-permission-toolbar]')):null,domains,ruleGroups,clippedModes,horizontalOverflow:document.documentElement.scrollWidth>innerWidth+1};
    })()`)
    densityMeasurements.push({ name, ...measurement })
    await fs.writeFile(
      path.join(evidence, 'web-permissions-density.json'),
      JSON.stringify(densityMeasurements, null, 2),
    )
    await screenshot(name)
    check(measurement.dark === (theme === 'dark'), name + ': requested theme is rendered')
    check(!measurement.horizontalOverflow && (await noOverflow()), name + ': no horizontal overflow')
    check(measurement.clippedModes.length === 0, name + ': permission values are readable without ellipsis')
    const [sandbox, files, network, tools] = measurement.domains
    check(
      sandbox?.id === 'sandbox' &&
        sandbox.visible &&
        sandbox.y >= measurement.toolbar?.bottom &&
        sandbox.bottom <= measurement.ruleLayout.y &&
        sandbox.x === measurement.ruleLayout.x &&
        sandbox.width === measurement.ruleLayout.width,
      name + ': the full-width sandbox section is visible above the permission rules',
    )
    if (measurement.ruleLayout.width >= 928) {
      check(
        tools.x >= files.x + files.width && Math.abs(tools.y - files.y) <= 4 && network.y >= files.bottom,
        name + ': resource and tool domains share the wide workspace',
      )
    } else {
      check(
        files.x === network.x && network.x === tools.x && network.y >= files.bottom && tools.y >= network.bottom,
        name + ': permission domains stack in document order on narrow panes',
      )
    }
    const domainForKind = { filesystem: 'filesystem', network: 'network', 'tool-name': 'tools', command: 'tools' }
    check(
      measurement.ruleGroups.length === 4 &&
        measurement.ruleGroups.every(
          (group) =>
            group.heading &&
            group.domain === domainForKind[group.kind] &&
            group.rows.every(
              (row) =>
                row.kind === group.kind &&
                row.height > 0 &&
                row.x >= group.x &&
                row.x + row.width <= group.x + group.width + 1,
            ),
        ),
      name + ': all rule tables and rows remain inside their own permission domain',
    )
    check(
      await cdp.evaluate(`!document.querySelector('[data-permission-sections]')`),
      name + ': the toolbar omits the section shortcuts',
    )
    const minimumDefaults = { filesystem: 2, network: 3, tools: 2 }
    const fileDefaultsVisible = mobile
      ? files.defaults.some((item) => item.visible)
      : files.defaults.every((item) => item.visible)
    check(
      [files, network, tools].every((domain) => domain.defaults.length >= minimumDefaults[domain.id]) &&
        fileDefaultsVisible &&
        (mobile || measurement.ruleGroups.some((group) => group.rows.some((row) => row.visible))),
      name + ': the first viewport shows permission controls below the sandbox',
    )
    check(
      measurement.toolbar && measurement.toolbar.height <= (mobile ? 205 : 158),
      name + ': scope and save controls use a compact toolbar',
    )
  }
  async function exerciseWorkspaceSelection() {
    await navigate('/settings/permissions?view=access&scope=workspace')
    await wait(loaded, 'workspace access scope')
    check(
      (await visible(workspaceTrigger)) &&
        (await cdp.evaluate(`document.querySelector('[data-permission-target]')?.getAttribute('title') === '/fixture/project/.agena/agena.json'`)),
      'workspace scope defaults to the server file and offers a workspace chooser',
    )
    check(
      (await inheritedControlsLocked(['path.workspace', 'path.external', 'tools.default', 'tools.read-only'])) &&
        (await compactSourcesMatch('Inherit', 'This scope', 'Global layer')),
      'workspace inherited values are disabled and source controls show only their two choices',
    )
    await makeLocal('tools.default')
    await select(defaultTrigger('tools.default'), 'Allow')
    await save()
    check(
      fixture.values.workspace.tools.default === 'allow' && fixture.values.global.tools.default === 'deny',
      'workspace saves stay isolated from global permission changes',
    )
    await select(defaultTrigger('tools.default'), 'Deny')
    const workspaceWrites = fixture.writes.length
    await chooseWorkspace('other-project', 'other-project')
    await wait(() => visible('[role="dialog"]'), 'switching workspaces protects an unsaved policy')
    await cdp.clickText('[role="dialog"] button', 'Keep editing')
    await wait(async () => !(await visible('[role="dialog"]')), 'cancelled workspace switch finishes')
    check(
      (await visible('[data-permission-savebar]')) &&
        !(await cdp.evaluate(`new URLSearchParams(location.search).has('targetWorkspaceId')`)) &&
        fixture.writes.length === workspaceWrites,
      'cancelling workspace selection retains the server workspace and its draft',
    )
    await chooseWorkspace('other-project', 'other-project')
    await wait(() => visible('[role="dialog"]'), 'select the same workspace again')
    await cdp.clickText('[role="dialog"] button', 'Discard changes')
    await wait(
      async () => (await loaded()) && (await cdp.evaluate(`document.querySelector('[data-permission-target]')?.title === '/fixture/other-project/.agena/agena.json'`)),
      'a manual selection loads the other workspace configuration file',
    )
    await wait(() => cdp.evaluate(`document.querySelector('[data-saved-rules]')?.innerText.includes('workspace.eight.fixture')`), 'selected workspace approval rules load')
    check(
      fixture.workspaceCatalogRequests.includes('other-project') &&
        (await defaultChoicesMatch({ 'tools.default': 'Ask' })) &&
        (await cdp.evaluate(`new URLSearchParams(location.search).get('targetWorkspaceId') === '8' && !document.querySelector('[data-saved-rules]').innerText.includes('workspace.seven.fixture')`)),
      'searching the server catalog selects the other workspace and loads only its rules',
    )
    await select(defaultTrigger('tools.default'), 'Deny')
    await save()
    check(
      fixture.writes.at(-1).workspaceId === 8 && fixture.workspaceValues[8].tools.default === 'deny' &&
        fixture.values.workspace.tools.default === 'allow',
      'saving after switching writes only to the selected workspace file',
    )
    await cdp.click(sourceChoice('tools.default', 'inherit'))
    await save()
    check(
      fixture.writes.at(-1).workspaceId === 8 && JSON.stringify(fixture.workspaceValues[8]) === '{}' &&
        fixture.values.workspace.tools.default === 'allow',
      'restoring inheritance clears the selected workspace file override',
    )
    await cdp.click('[data-saved-rules] button[aria-label="Create tool rule"]')
    await cdp.fill('[data-saved-rules] input[placeholder="agena.shell.exec"]', 'agena.workspace.fixture')
    await cdp.clickText('[data-saved-rules] button', 'Create rule')
    await wait(() => fixture.ruleWrites.at(-1)?.tool_name === 'agena.workspace.fixture', 'new approval is saved')
    check(
      fixture.ruleWrites.at(-1).workspace_id === 8 && fixture.rules.some((rule) => rule.workspace_id === 8 && rule.tool_name === 'agena.workspace.fixture'),
      'creating an approval uses the same workspace as the editor',
    )
    await chooseWorkspace('Server workspace', 'server workspace')
    await wait(
      async () => (await loaded()) && !(await cdp.evaluate(`new URLSearchParams(location.search).has('targetWorkspaceId')`)),
      'the chooser can return to the server workspace',
    )
  }
  async function exerciseMobileWorkspaceSelection() {
    await cdp.evaluate("window.__botProbe.setAppLocale('zh-CN')")
    await cdp.send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true })
    await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 1 })
    await frame()
    await navigate('/settings/permissions?view=access&scope=workspace')
    await wait(loaded, 'mobile workspace permissions')
    await chooseWorkspace('other-project', 'other-project')
    await wait(
      async () => (await loaded()) && (await cdp.evaluate(`document.querySelector('[data-permission-target]')?.title === '/fixture/other-project/.agena/agena.json'`)),
      'mobile workspace selection loads the correct file',
    )
    check(
      (await noOverflow()) && (await defaultSourcesMatch('inherit')),
      'mobile workspace selection preserves the compact permission header and inherited values',
    )
    await pageTop()
    await screenshot('web-permissions-workspace-mobile')
  }
  await cdp.send('Page.navigate', { url: base + '/chat?sessionId=7' })
  await wait(() => cdp.evaluate(`window.__botProbe?.activeComposerSession()==='7'`), 'current project conversation')
  await cdp.evaluate(`window.__botProbe.setAppLocale('en-US')`)
  await cdp.evaluate(`window.__botProbe.applyAppearanceSettingsToDom({useSystemTheme:false,themeVariant:'dark'})`)
  if (workspaceOnly) {
    await exerciseWorkspaceSelection()
    await exerciseMobileWorkspaceSelection()
    return
  }
  await navigate('/settings/permissions?view=filesystem&scope=global&sessionId=999')
  await wait(loaded, 'all permission sections on a single page')
  check(
    await cdp.evaluate(
      `!document.querySelector('.permission-studio [data-setting-source], .permission-studio [data-setting-source-choice]') && document.querySelectorAll('.permission-studio [data-setting-reset]').length >= 9`,
    ),
    'global settings show direct values and separate reset actions without any default/local selector',
  )
  check(
    await cdp.evaluate(
      `!document.querySelector('.permission-studio .rule-table-source, .permission-studio .rule-table-mobile-source, .permission-studio .rule-table-source-column')`,
    ),
    'global permission rule tables omit source columns and source badges',
  )
  check(
    await cdp.evaluate(
      `document.querySelector('[data-permission-default="path.workspace"] [data-setting-reset]').disabled`,
    ),
    'a missing global field shows its default value with reset disabled',
  )
  await select(defaultTrigger('path.workspace'), 'Deny')
  check(await visible('[data-permission-savebar]'), 'directly editing a default global value creates a normal draft')
  await cdp.click('[data-permission-default="path.workspace"] [data-setting-reset]')
  check(
    (await defaultChoicesMatch({ 'path.workspace': 'Read and write' })) &&
      !(await visible('[data-permission-savebar]')),
    'restoring one global field returns to its default without leaving a source-state draft',
  )
  await cdp.click('[data-permission-default="network.default"] [data-setting-reset]')
  check(
    (await defaultChoicesMatch({ 'network.default': 'Allow', 'network.inbound': 'Deny' })) &&
      (await groupCounts()).network === 2,
    'restoring global outbound uses the built-in value and preserves both configured rule lists',
  )
  await select(defaultTrigger('network.default'), 'Deny')
  check(
    !(await visible('[data-permission-savebar]')) && fixture.writes.length === 0,
    'returning to the saved global value is clean and causes no write',
  )
  check(
    await cdp.evaluate(
      `new URLSearchParams(location.search).get('view')==='access' && new URLSearchParams(location.search).get('scope')==='global' && !new URLSearchParams(location.search).has('sessionId') && location.hash==='#permission-filesystem' && window.__botProbe.chat.selectedSessionId==='7'`,
    ),
    'old file URL becomes access and keeps global scope without changing the current conversation',
  )
  check(
    await cdp.evaluate(
      `document.querySelectorAll('[data-keyboard-item="security/access"]').length===1 && !document.querySelector('[data-keyboard-item="security/files"],[data-keyboard-item="security/network"],[data-keyboard-item="security/execution"]')`,
    ),
    'sidebar has one access editor for all three permission domains',
  )
  check(
    await cdp.evaluate(
      `document.querySelectorAll('.permission-studio').length===1 && [...document.querySelectorAll('.permission-domain[data-permission-section]')].every(section=>section.querySelector('h2 a[href="#permission-'+section.dataset.permissionSection+'"]'))`,
    ),
    'three domain headings provide section links within one editor',
  )
  check(
    await cdp.evaluate(
      `(() => { const groups=[...document.querySelectorAll('[data-permission-rule-table]')]; return groups.map(group=>group.dataset.permissionRuleTable).join('|')==='filesystem|network|tool-name|command' && groups.every(group=>[...group.querySelectorAll('[data-permission-rule]')].every(row=>row.dataset.permissionRule===group.dataset.permissionRuleTable)) && !document.querySelector('.rule-table-filters'); })()`,
    ),
    'file, network, tool name, and command rules use four distinct tables without a mixed-rule filter',
  )
  check(
    JSON.stringify(await groupCounts()) === JSON.stringify({ filesystem: 2, network: 2, 'tool-name': 4, command: 3 }),
    'each rule type retains its complete local and inherited rules',
  )
  check(
    await cdp.evaluate(
      `!document.querySelector('[data-permission-rule-table="filesystem"] .rule-table-condition, [data-permission-rule-table="tool-name"] .rule-table-condition, .rule-table-type') && !!document.querySelector('[data-permission-rule-table="network"] .rule-table-condition') && !document.querySelector('[data-permission-rule-table="command"] .rule-table-condition') && [...document.querySelectorAll('[data-permission-rule-table="command"] th')].some(cell=>cell.textContent.trim()==='Command pattern')`,
    ),
    'each table shows only columns meaningful for its rule type',
  )
  await cdp.fill(`${ruleGroup('filesystem')} input[aria-label$="Search rules…"]`, 'docs')
  await frame()
  check(
    JSON.stringify(await groupCounts()) === JSON.stringify({ filesystem: 1, network: 2, 'tool-name': 4, command: 3 }),
    'file search only filters the file rules group',
  )
  await cdp.fill(`${ruleGroup('tool-name')} input[aria-label$="Search rules…"]`, 'fetch')
  await frame()
  check(
    JSON.stringify(await groupCounts()) === JSON.stringify({ filesystem: 1, network: 2, 'tool-name': 1, command: 3 }),
    'tool-name search preserves the independent file search and other rule groups',
  )
  await cdp.click(`${ruleGroup('filesystem')} input[aria-label="Select all permission rules"]`)
  await cdp.click(`${ruleGroup('tool-name')} input[aria-label="Select all permission rules"]`)
  check(
    await cdp.evaluate(
      `['filesystem','tool-name'].every(kind=>document.querySelector('[data-permission-rule-table="'+kind+'"] .rule-table-bulk')) && !document.querySelector('[data-permission-rule-table="command"] .rule-table-bulk') && [...document.querySelectorAll('[data-permission-row-action="select"]:checked')].length===2`,
    ),
    'select-all and bulk controls are scoped to each independent rule group',
  )
  const toolBulk = ruleGroup('tool-name') + ' .permission-bulk-actions'
  check(await bulkFits(toolBulk, false), 'tool-name bulk actions fit in a single row with one selection count')
  await screenshot('web-permissions-tool-bulk-desktop')
  await cdp.click(`${ruleGroup('filesystem')} button[aria-label="Select none"]`)
  check(
    await cdp.evaluate(
      `!document.querySelector('[data-permission-rule-table="filesystem"] .rule-table-bulk') && !!document.querySelector('[data-permission-rule-table="tool-name"] .rule-table-bulk') && [...document.querySelectorAll('[data-permission-row-action="select"]:checked')].length===1`,
    ),
    'clearing file selection leaves tool-name selection intact',
  )
  await cdp.click(`${ruleGroup('tool-name')} button[aria-label="Select none"]`)
  await cdp.fill(`${ruleGroup('filesystem')} input[aria-label$="Search rules…"]`, '')
  await cdp.fill(`${ruleGroup('tool-name')} input[aria-label$="Search rules…"]`, '')
  await frame()
  for (const [kind, target] of [
    ['filesystem', '<state>'],
    ['tool-name', 'agena.web.search'],
  ]) {
    await select(rule(kind, target) + ' [data-permission-mode] button[aria-haspopup="dialog"]', 'Deny')
    check(
      await cdp.evaluate(
        'document.querySelector(' + JSON.stringify(rule(kind, target)) + ')?.dataset.ruleScope==="local"',
      ),
      kind + ' built-in rule can be edited directly without first choosing a source',
    )
    await cdp.click(rule(kind, target) + ' [data-permission-row-action="reset"]')
    check(
      (await cdp.evaluate(
        'document.querySelector(' + JSON.stringify(rule(kind, target)) + ')?.dataset.ruleScope==="inherited"',
      )) && !(await visible('[data-permission-savebar]')),
      kind + ' reset restores its built-in value and preserves the other rules',
    )
  }
  const inheritedCommand = rule('command', 'routine')
  const commandReset = ruleGroup('command') + ' button[data-setting-reset]'
  await select(inheritedCommand + ' [data-permission-mode] button[aria-haspopup="dialog"]', 'Deny')
  check(
    await cdp.evaluate(`(() => {
      const rows=[...document.querySelectorAll('[data-permission-rule="command"]')];
      return rows.length===3 && rows.every(row=>row.dataset.ruleScope==='local');
    })()`),
    'directly editing a built-in command copies the shared rule table',
  )
  await cdp.click(commandReset)
  await wait(() => visible('[role="dialog"]'), 'shared command reset explains its scope')
  check(
    await cdp.evaluate(
      `document.querySelector('[role="dialog"]').textContent.includes('This removes all command rules defined in this layer.')`,
    ),
    'global command reset identifies the affected rule table',
  )
  await cdp.clickText('[role="dialog"] button', 'Cancel', true)
  check(
    await cdp.evaluate(`document.querySelector(${JSON.stringify(inheritedCommand)})?.dataset.ruleScope==='local'`),
    'cancelling the reset preserves local command rules',
  )
  await cdp.click(commandReset)
  await wait(() => visible('[role="dialog"]'), 'confirm command reset')
  await cdp.clickText('[role="dialog"] button', 'Reset to defaults', true)
  await wait(async () => !(await visible('[data-permission-savebar]')), 'command reset restores the original layer')
  check(
    await cdp.evaluate(`document.querySelector(${JSON.stringify(inheritedCommand)})?.dataset.ruleScope==='inherited'`),
    'confirmed global command reset restores the three built-in rules',
  )
  await cdp.click('[data-permission-reset-all]')
  await wait(() => visible('[role="dialog"]'), 'global permission reset explains its page scope')
  check(
    await cdp.evaluate(
      `document.querySelector('[role="dialog"]').textContent.includes('Restore default global permissions?')`,
    ),
    'global permission reset is a restore-default action with an explicit scope',
  )
  await cdp.clickText('[role="dialog"] button', 'Reset to defaults', true)
  await cdp.clickText('.permission-summary', 'Network policy JSON')
  check(
    await cdp.evaluate(
      `JSON.parse(document.querySelector('textarea[aria-label="Network policy JSON"]').value).version === 2`,
    ),
    'global network JSON is directly available when no network fields are configured',
  )
  await cdp.fill('textarea[aria-label="Network policy JSON"]', '{}')
  await cdp.clickText('.permission-json-body button', 'Apply JSON', true)
  check(
    (await defaultChoicesMatch({
      'network.default': 'Deny',
      'network.inbound': 'Deny',
      'network.hostLoopback': 'Deny',
    })) && fixture.writes.length === 0,
    'an explicit offline JSON policy can be drafted without selecting a local source',
  )
  check(
    await cdp.evaluate(`(() => {
      const domain = document.querySelector('[data-permission-section="network"]');
      return !domain.querySelector('.rule-table-empty, [data-permission-default="network.allow"], [data-permission-default="network.deny"]') &&
        !/Block external network|Allow outbound by default/.test(domain.innerText) &&
        !!domain.querySelector('[data-permission-create-toggle="network"]');
    })()`),
    'an empty network table keeps its add action without the summary, empty message or presets',
  )
  await cdp.clickText('[data-permission-savebar] button', 'Discard changes')
  await cdp.clickText('.permission-summary', 'Network policy JSON')
  await pageTop()
  check(
    !(await visible('[data-permission-savebar]')),
    'discarding a global reset and JSON edit restores the saved policy',
  )
  await captureDensity('web-permissions-desktop-dark')
  await cdp.evaluate(`window.__botProbe.applyAppearanceSettingsToDom({useSystemTheme:false,themeVariant:'light'})`)
  await captureDensity('web-permissions-desktop-light', false, 'light')
  await cdp.evaluate(`window.__botProbe.applyAppearanceSettingsToDom({useSystemTheme:false,themeVariant:'dark'})`)
  await cdp.click('[data-permission-rule-link="command"]')
  await wait(
    () => cdp.evaluate(`document.activeElement?.dataset.permissionRuleTable==='command'`),
    'command rule count link focuses its own table',
  )
  await frame()
  check(
    await cdp.evaluate(
      `document.querySelector('[data-permission-rule-table="command"] [data-permission-rule-heading]').getBoundingClientRect().top >= document.querySelector('[data-permission-toolbar]').getBoundingClientRect().bottom && document.querySelectorAll('[data-permission-rule-table]').length===4 && location.hash==='#permission-rules-command'`,
    ),
    'rule count links navigate to a visible group heading without filtering away other types',
  )
  await screenshot('web-permissions-command-rules')

  await select('[data-permission-default="path.workspace"] button[aria-haspopup="dialog"]', 'Read only')
  await select(defaultTrigger('tools.default'), 'Deny')
  await select(
    '[data-permission-default="tools.read-only"] button[aria-haspopup="dialog"]',
    'Deny · Follow tool default',
  )
  check(
    await cdp.evaluate(
      `document.querySelector('[data-permission-default="tools.read-only"] button[aria-haspopup="dialog"]')?.innerText.trim()==='Deny · Follow tool default'`,
    ),
    'read-only Auto displays the actual value and its follow-default behavior together',
  )
  await captureDensity('web-permissions-follow-default')
  await select(
    `${rule('filesystem', '<workspace>/docs')} [data-permission-mode] button[aria-haspopup="dialog"]`,
    'Deny',
  )
  await select(`${rule('tool-name', 'agena.web.fetch')} [data-permission-mode] button[aria-haspopup="dialog"]`, 'Allow')
  const commandForm = ruleGroup('command') + ' form'
  await cdp.click(ruleGroup('command') + ' [data-permission-create-toggle="command"]')
  check(
    await cdp.evaluate(`!document.querySelector(${JSON.stringify(commandForm + ' [title="Shell-capable tool"]')})`),
    'a command pattern can be added directly',
  )
  await cdp.fill(commandForm + ' input[aria-label="Command pattern"]', 'git status')
  await select(commandForm + ' button[aria-haspopup="dialog"]', 'Deny')
  await cdp.click(commandForm + ' button[type="submit"]')
  await cdp.fill(commandForm + ' input[aria-label="Command pattern"]', 'git push *')
  await select(commandForm + ' button[aria-haspopup="dialog"]', 'Ask')
  await cdp.click(commandForm + ' button[type="submit"]')
  await cdp.click(`${rule('network', '198.51.100.24')} [data-permission-row-action="edit"]`)
  await wait(() => visible('[data-permission-network-ports]'), 'network rule editor opens from the unified table')
  await cdp.fill('[data-permission-network-ports]', '8443')
  await cdp.click('[data-permission-network-apply]')
  await wait(
    async () => !(await visible('[data-permission-network-ports]')),
    'edited network rule returns to the table',
  )
  check(
    await cdp.evaluate(
      `[...document.querySelectorAll('[data-permission-rule][data-rule-scope="inherited"]:not([data-permission-rule="network"])')].every(row => !!row.querySelector('[data-permission-mode] button[aria-haspopup="dialog"]')) && !document.querySelector('[data-permission-row-action="source-local"],[data-permission-row-action="source-inherit"]')`,
    ),
    'global built-in rules are directly editable and never expose a copy-to-local source action',
  )
  check(fixture.writes.length === 0, 'file, network, and tool edits share an unsaved draft')
  await cdp.click(`${section('network')} h2 a[href="#permission-network"]`)
  check(!(await visible('[role="dialog"]')), 'in-page section navigation keeps the draft without a discard dialog')
  await cdp.evaluate(`void ${navigation('/settings/permissions?view=sandbox&scope=global')}`)
  await wait(
    () =>
      cdp.evaluate(
        `location.hash === '#permission-sandbox' && document.activeElement?.dataset.permissionSection === 'sandbox'`,
      ),
    'the sandbox bookmark focuses the independent safeguards section',
  )
  check(await visible('[data-permission-savebar]'), 'the sandbox bookmark keeps the existing permission draft')
  for (const destination of [
    'view=access&scope=workspace',
    'view=overview&scope=global',
    'view=profiles&scope=global',
  ]) {
    await cdp.evaluate(`void ${navigation('/settings/permissions?' + destination)}`)
    await wait(() => visible('[role="dialog"]'), 'unsaved permission confirmation')
    await cdp.clickText('[role="dialog"] button', 'Keep editing')
    await wait(async () => !(await visible('[role="dialog"]')), 'continue editing after navigation is cancelled')
    check(
      await cdp.evaluate(
        `new URLSearchParams(location.search).get('view')==='access' && new URLSearchParams(location.search).get('scope')==='global' && !!document.querySelector('[data-permission-savebar]')`,
      ),
      `cancelling ${destination} retains the complete draft and original scope`,
    )
  }
  await cdp.click(`[data-saved-rules] button[aria-label="Create tool rule"]`)
  await cdp.fill(`[data-saved-rules] input[placeholder="agena.shell.exec"]`, 'agena.shell.exec')
  await cdp.clickText(`[data-saved-rules] button`, 'Create rule')
  await wait(() => fixture.ruleWrites.length === 1, 'remembered tool rule saves independently')
  check(
    fixture.writes.length === 0 && (await visible('[data-permission-savebar]')),
    'saving a remembered rule preserves all pending access changes',
  )
  await save()
  check(
    fixture.writes.length === 1 &&
      fixture.values.global.path.workspace === 'readonly' &&
      fixture.values.global.network.default === 'deny' &&
      fixture.values.global.tools.default === 'deny',
    'one global mutation persists file, network, and tool changes together',
  )
  check(
    fixture.rules.some((rule) => rule.tool_name === 'agena.shell.exec' && rule.scope === 'global'),
    'combined save preserves the remembered rule and its scope',
  )
  check(
    fixture.values.global.path.rules['<workspace>/docs'] === 'deny' &&
      fixture.values.global.network.allow[0].ports[0] === 8443 &&
      fixture.values.global.tools.names['agena.fs.read'] === 'allow' &&
      fixture.values.global.tools.names['agena.web.fetch'] === 'allow' &&
      fixture.values.global.tools.rules['*']['read-only'] === 'auto' &&
      fixture.values.global.tools.rules['shell.*']['git status'] === 'deny' &&
      fixture.values.global.tools.rules['shell.*']['git push *'] === 'ask' &&
      Object.keys(fixture.values.global.tools.rules['shell.*']).join('|') === 'no-op|routine|dangerous|git status|git push *' &&
      !Object.hasOwn(fixture.values.global.path.rules, '<state>'),
    'type-separated table edits save exact path, network, name, and command keys while preserving rule order and inheritance',
  )

  for (const oldView of ['network', 'tools']) {
    await navigate(`/settings/permissions?view=${oldView}&scope=global`)
    await wait(
      () =>
        cdp.evaluate(
          `new URLSearchParams(location.search).get('view')==='access' && location.hash===${JSON.stringify('#permission-' + oldView)} && document.activeElement?.dataset.permissionSection===${JSON.stringify(oldView)}`,
        ),
      `${oldView} alias scrolls and focuses its section`,
    )
  }
  check(true, 'old network and tool links focus their sections on the canonical access page')
  await exerciseWorkspaceSelection()

  await navigate('/settings/permissions?view=tools&scope=session&sessionId=999')
  await wait(loaded, 'session access scope')
  const inheritedChoices = {
    'path.workspace': 'Read only',
    'path.external': 'Deny',
    'tools.default': 'Allow',
    'tools.read-only': 'Allow · Follow tool default',
  }
  const explicitChoices = {
    'path.workspace': 'Read only',
    'path.external': 'Deny',
    'tools.default': 'Allow',
    'tools.read-only': 'Allow',
  }
  check(
    (await defaultChoicesMatch(inheritedChoices)) && (await defaultSourcesMatch('inherit')),
    'session defaults show the Inherit choice alongside their actual value',
  )
  check(
    (await inheritedControlsLocked(['path.workspace', 'path.external', 'tools.default', 'tools.read-only'])) &&
      (await compactSourcesMatch('Inherit', 'This scope', 'Parent configuration')),
    'session inherited values are read only and parent details stay in the source tooltip',
  )
  const networkKeys = ['network.default', 'network.inbound', 'network.hostLoopback', 'network.allow', 'network.deny']
  check(
    (await withNetworkSources(() => defaultSourcesMatch('inherit', networkKeys))) &&
      (await cdp.evaluate(`['network.default','network.inbound','network.hostLoopback'].every(key=>
        document.querySelector('[data-permission-default="'+key+'"] button[aria-haspopup="dialog"]')?.disabled
      )`)),
    'all five network settings keep independent source controls and inherited scalar values are disabled',
  )
  const writesBeforeInheritance = fixture.writes.length
  await cdp.evaluate(
    `document.querySelector('[data-permission-default="path.workspace"] button[aria-haspopup="dialog"]').click()`,
  )
  check(!(await visible('[data-keyboard-overlay="true"]')), 'clicking an inherited value cannot open its editor')
  check(
    fixture.writes.length === writesBeforeInheritance &&
      JSON.stringify(fixture.values.session) === '{}' &&
      !(await visible('[data-permission-savebar]')),
    'attempting to edit an inherited value does not materialize a session override',
  )
  await cdp.click(sourceChoice('path.workspace', 'local'))
  check(
    (await defaultSourcesMatch('local', ['path.workspace'])) &&
      (await defaultChoicesMatch({ 'path.workspace': 'Read only' })) &&
      (await visible('[data-permission-savebar]')),
    'one source click pins the inherited value in this layer without changing its value',
  )
  await cdp.click(sourceChoice('path.workspace', 'inherit'))
  check(
    (await defaultSourcesMatch('inherit', ['path.workspace'])) && !(await visible('[data-permission-savebar]')),
    'one source click restores inheritance without leaving a meaningless draft',
  )
  const networkParent = clone(fixture.values.global.network)
  await makeLocal('network.default')
  await select(defaultTrigger('network.default'), 'Allow')
  check(
    (await defaultSourcesMatch('local', ['network.default'])) &&
      (await withNetworkSources(() => defaultSourcesMatch(
        'inherit',
        networkKeys.filter((key) => key !== 'network.default'),
      ))),
    'editing outbound creates only an outbound override',
  )
  await save()
  check(
    JSON.stringify(fixture.values.session.network) === JSON.stringify({ version: 2, default: 'allow' }),
    'the first scalar network edit stores one v2 field and leaves other defaults and rules inherited',
  )
  await cdp.click(sourceChoice('network.inbound', 'local'))
  await cdp.click(sourceChoice('network.hostLoopback', 'local'))
  await save()
  check(
    fixture.values.session.network.inbound === 'deny' &&
      fixture.values.session.network.hostLoopback === 'deny' &&
      !Object.hasOwn(fixture.values.session.network, 'allow') &&
      !Object.hasOwn(fixture.values.session.network, 'deny'),
    'equal inbound and host-loopback values can each be pinned without pinning either rule list',
  )
  await cdp.click(sourceChoice('network.default', 'inherit'))
  await save()
  check(
    !Object.hasOwn(fixture.values.session.network, 'default') &&
      fixture.values.session.network.inbound === 'deny' &&
      fixture.values.session.network.hostLoopback === 'deny' &&
      (await defaultChoicesMatch({ 'network.default': 'Deny' })),
    'restoring outbound inheritance preserves the other local network fields',
  )
  await cdp.click(sourceChoice('network.inbound', 'inherit'))
  await cdp.click(sourceChoice('network.hostLoopback', 'inherit'))
  await save()
  check(
    JSON.stringify(fixture.values.session) === '{}',
    'restoring the last local field removes the empty network object',
  )

  await setNetworkListSource('network.allow', 'local')
  await save()
  check(
    JSON.stringify(fixture.values.session.network.allow) === JSON.stringify(networkParent.allow) &&
      Object.keys(fixture.values.session.network).length === 2 &&
      (await withNetworkSources(() => defaultSourcesMatch('inherit', [
        'network.default',
        'network.inbound',
        'network.hostLoopback',
        'network.deny',
      ]))),
    'pinning the allow list stores that list alone',
  )
  check(
    await cdp.evaluate(`(() => {
      const row = document.querySelector(${JSON.stringify(rule('network', '192.0.2.13'))});
      return !!row && [...row.querySelectorAll('[data-permission-row-action="edit"],[data-permission-row-action="remove"]')].every(button => button.disabled);
    })()`),
    'the inherited deny list cannot be edited or removed before selecting this scope',
  )
  await setNetworkListSource('network.deny', 'local')
  await cdp.click(`${rule('network', '192.0.2.13')} [data-permission-row-action="remove"]`)
  await save()
  check(
    Array.isArray(fixture.values.session.network.deny) &&
      fixture.values.session.network.deny.length === 0 &&
      JSON.stringify(fixture.values.session.network.allow) === JSON.stringify(networkParent.allow) &&
      (await withNetworkSources(() => defaultSourcesMatch('local', ['network.allow', 'network.deny']))),
    'deleting the last rule after explicitly selecting this scope keeps an empty local deny list and preserves allow rules',
  )
  await setNetworkListSource('network.allow', 'inherit')
  await save()
  check(
    !Object.hasOwn(fixture.values.session.network, 'allow') &&
      fixture.values.session.network.deny.length === 0 &&
      (await withNetworkSources(async () =>
        (await defaultSourcesMatch('inherit', ['network.allow'])) &&
        (await defaultSourcesMatch('local', ['network.deny'])),
      )),
    'allow-list inheritance can be restored while deny remains explicitly empty',
  )
  await setNetworkListSource('network.deny', 'inherit')
  await save()
  check(
    JSON.stringify(fixture.values.session) === '{}' &&
      JSON.stringify(fixture.values.global.network) === JSON.stringify(networkParent),
    'restoring the remaining list resumes the parent policy without changing the parent',
  )
  await captureDensity('web-permissions-session-inherited')
  for (const [key, label] of Object.entries(explicitChoices)) {
    await makeLocal(key)
    await select(defaultTrigger(key), label)
  }
  check(
    (await defaultChoicesMatch(explicitChoices)) &&
      (await defaultSourcesMatch('local')) &&
      (await visible('[data-permission-savebar]')),
    'explicit modes equal to the parent remain distinguishable from inheritance',
  )
  await save()
  check(
    fixture.writes.at(-1).sessionId === 7 &&
      fixture.values.session.path.workspace === 'readonly' &&
      fixture.values.session.path.external === 'deny' &&
      fixture.values.session.tools.default === 'allow' &&
      fixture.values.session.tools.rules['*']['read-only'] === 'allow' &&
      !Object.hasOwn(fixture.values.session, 'network'),
    'saving equal-value overrides persists only the explicit session choices',
  )
  for (const key of Object.keys(inheritedChoices)) await cdp.click(sourceChoice(key, 'inherit'))
  await save()
  check(
    fixture.writes.at(-1).sessionId === 7 && JSON.stringify(fixture.values.session) === '{}',
    'restoring inheritance removes file and tool overrides from the saved session payload',
  )
  await navigate('/settings/permissions?view=access&scope=workspace')
  await wait(loaded, 'leave restored session policy')
  await navigate('/settings/permissions?view=access&scope=session')
  await wait(loaded, 'reload restored session policy')
  check(
    (await defaultChoicesMatch(inheritedChoices)) &&
      (await defaultSourcesMatch('inherit')) &&
      !(await visible('[data-permission-savebar]')),
    'reloaded session selectors preserve inheritance and remain clean',
  )
  await makeLocal('path.workspace')
  await select('[data-permission-default="path.workspace"] button[aria-haspopup="dialog"]', 'Read only')
  await makeLocal('network.default')
  await makeLocal('tools.default')
  await select(defaultTrigger('tools.default'), 'Auto')
  await save()
  const sessionWrite = fixture.writes.at(-1)
  check(
    sessionWrite.scope === 'session' &&
      sessionWrite.sessionId === 7 &&
      sessionWrite.value.path.workspace === 'readonly' &&
      sessionWrite.value.network.default === 'deny' &&
      sessionWrite.value.tools.default === 'auto',
    'one session mutation saves all three domains to the last conversation instead of the obsolete URL target',
  )
  const originalSessionPermission = clone(fixture.values.session)
  const recentSessionsBeforePicker = await cdp.evaluate('window.__botProbe.chat.sessions.map(session=>session.id)')
  await select('[data-permission-default="path.workspace"] button[aria-haspopup="dialog"]', 'Deny')
  check(await visible('[data-permission-savebar]'), 'session 7 has an unsaved edit before changing the picker target')
  const writesBeforeSwitch = fixture.writes.length
  await chooseSession('Other project conversation', true)
  await wait(
    () =>
      cdp.evaluate(
        `[...document.querySelectorAll('[role="dialog"] button')].some(button=>button.innerText.trim()==='Keep editing')`,
      ),
    'session switch asks about the unsaved permission draft',
  )
  await cdp.clickText('[role="dialog"] button', 'Keep editing')
  await wait(async () => !(await visible('[role="dialog"]')), 'cancelled session switch closes its confirmation')
  check(
    (await cdp.evaluate(
      `!new URLSearchParams(location.search).has('targetSessionId') && document.querySelector('[data-permission-default="path.workspace"] button[aria-haspopup="dialog"]').innerText.trim()==='Deny' && document.querySelector('[data-permission-session-picker]').innerText.includes('Existing project conversation')`,
    )) && fixture.writes.length === writesBeforeSwitch,
    'cancelling a session switch keeps the original target, header and draft without writing',
  )
  await chooseSession('Other project conversation', true)
  await wait(
    () =>
      cdp.evaluate(
        `[...document.querySelectorAll('[role="dialog"] button')].some(button=>button.innerText.trim()==='Discard changes')`,
      ),
    'confirmed session switch offers explicit draft discard',
  )
  await cdp.clickText('[role="dialog"] button', 'Discard changes')
  await waitSession(8)
  check(
    await cdp.evaluate(
      `document.querySelector('[data-permission-default="tools.default"] button[aria-haspopup="dialog"]').innerText.trim()==='Ask' && document.querySelector('[data-permission-session-picker]').innerText.includes('Other project conversation')`,
    ),
    'choosing another conversation loads that session permission and updates the same dropdown label',
  )
  await select('[data-permission-default="path.workspace"] button[aria-haspopup="dialog"]', 'Read only')
  await select('[data-permission-default="tools.default"] button[aria-haspopup="dialog"]', 'Allow')
  await save()
  check(
    fixture.writes.at(-1).sessionId === 8 &&
      fixture.sessionValues[8].path.workspace === 'readonly' &&
      fixture.sessionValues[8].tools.default === 'allow' &&
      JSON.stringify(fixture.values.session) === JSON.stringify(originalSessionPermission),
    'saving selected session 8 updates its policy and leaves the original conversation unchanged',
  )
  await select('[data-permission-scope] button[aria-haspopup="dialog"]', 'Effective Permission')
  await waitSession(8)
  await wait(
    () => cdp.evaluate(`document.querySelector('[data-saved-rules]').innerText.includes('workspace.eight.fixture')`),
    'effective remembered rules resolve the selected conversation workspace',
  )
  check(
    await cdp.evaluate(
      `document.querySelector('[data-saved-rules]').innerText.includes('session.eight.fixture') && !document.querySelector('[data-saved-rules]').innerText.includes('workspace.seven.fixture')`,
    ),
    'effective rules follow selected session 8 and its workspace instead of the active chat workspace',
  )
  await select('[data-permission-scope] button[aria-haspopup="dialog"]', 'Current Session Permission')
  await waitSession(8)
  await openSessionPicker()
  await cdp.fill(pickerSearch, '')
  await wait(
    () => visible('[data-keyboard-overlay="true"] button[aria-label="Next page"]'),
    'session catalog has real pagination',
  )
  await screenshot('web-permissions-session-picker')
  const catalogBeforePage = fixture.catalogRequests.length
  await cdp.click('[data-keyboard-overlay="true"] button[aria-label="Next page"]')
  await wait(
    () => fixture.catalogRequests.slice(catalogBeforePage).some((request) => request.offset > 0),
    'picker requests the next server session page',
  )
  await cdp.fill(pickerSearch, 'Older project conversation')
  await wait(
    () =>
      cdp.evaluate(
        `[...document.querySelectorAll('[data-option-menu-item] .font-medium')].some(item=>item.innerText.trim()==='Older project conversation')`,
      ),
    'server search finds a conversation outside the recently loaded page',
  )
  check(
    fixture.catalogRequests.some((request) => request.query === 'older project conversation'),
    'session search is served by the catalog instead of only the recent-chat cache',
  )
  await cdp.clickText('[data-option-menu-item] .font-medium', 'Older project conversation', true)
  await waitSession(164)
  await makeLocal('tools.default')
  await select('[data-permission-default="tools.default"] button[aria-haspopup="dialog"]', 'Deny')
  await save()
  check(
    fixture.writes.at(-1).sessionId === 164 && fixture.sessionValues[164].tools.default === 'deny',
    'an older selected session is editable even when it is absent from the recent list',
  )
  await chooseSession('Atlas')
  await waitSession(42)
  await makeLocal('tools.default')
  await select('[data-permission-default="tools.default"] button[aria-haspopup="dialog"]', 'Ask')
  await save()
  check(
    fixture.writes.at(-1).sessionId === 42 && fixture.sessionValues[42].tools.default === 'ask',
    'an existing Bot canonical session can be selected and saved without opening its chat',
  )
  check(
    (await cdp.evaluate(
      `window.__botProbe.chat.selectedSessionId==='7' && window.__botProbe.lastConversation()==='7'`,
    )) &&
      JSON.stringify(await cdp.evaluate('window.__botProbe.chat.sessions.map(session=>session.id)')) ===
        JSON.stringify(recentSessionsBeforePicker),
    'browsing and saving other sessions preserves the active chat, last conversation and recent-list order',
  )
  await navigate('/settings/permissions?view=access&scope=session')
  await wait(loaded, 'return to the default recently used conversation')
  await navigate('/settings/permissions?view=access&scope=effective')
  await wait(loaded, 'effective permission scope')
  check(
    await cdp.evaluate(
      `['filesystem','network','tools'].every(section => {
        const defaults=[...document.querySelectorAll('[data-permission-section="'+section+'"] [data-permission-default]')];
        return defaults.length>0 && defaults.every(row=>row.innerText.trim() && [...row.querySelectorAll('button[aria-haspopup="dialog"],input,textarea')].every(control=>control.disabled||control.readOnly));
      }) && [...document.querySelectorAll('.permission-studio textarea')].every(input=>input.readOnly) && !document.querySelector('[data-permission-savebar]')`,
    ),
    'effective scope keeps all three domains read only',
  )
  check(
    await cdp.evaluate(
      `!document.querySelector('[data-permission-rule-table] .rule-table-action-column, [data-permission-rule-table] .rule-table-row-actions')`,
    ),
    'read-only rules omit the empty actions column',
  )

  fixture.values.global = { path: { external: 'readonly' } }
  fixture.values.workspace = {}
  fixture.values.session = {}
  fixture.rules.splice(0)
  await navigate('/settings/permissions?view=access&scope=workspace')
  await wait(loaded, 'reload sparse inherited policy fixture')
  await navigate('/settings/permissions?view=access&scope=effective')
  await wait(loaded, 'sparse effective policy fixture')
  await cdp.evaluate(`window.__botProbe.setAppLocale('zh-CN')`)
  for (const viewport of [
    { width: 1280, height: 800 },
    { width: 1920, height: 1080 },
  ]) {
    await cdp.send('Emulation.setDeviceMetricsOverride', { ...viewport, deviceScaleFactor: 1, mobile: false })
    await frame()
    await captureDensity('web-permissions-effective-' + viewport.width)
    await navigate('/settings/permissions?view=access&scope=session')
    await wait(loaded, 'Chinese session inheritance presentation')
    check(
      (await defaultChoicesMatch({
        'path.workspace': '读写',
        'path.external': '只读',
        'tools.default': '允许',
        'tools.read-only': '允许',
      })) && (await defaultSourcesMatch('inherit')) && (await compactSourcesMatch('继承', '本层', '上层配置')),
      `Chinese session selectors display inherited values at ${viewport.width}px`,
    )
    await captureDensity('web-permissions-session-inherited-' + viewport.width)
    await navigate('/settings/permissions?view=access&scope=effective')
    await wait(loaded, 'return to effective density view')
  }

  await navigate('/settings/permissions?view=access&scope=global')
  await wait(loaded, 'global scope for mobile')
  await cdp.send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true })
  await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 1 })
  await cdp.evaluate(`window.__botProbe.setAppLocale('zh-CN')`)
  await frame()
  await captureDensity('web-permissions-mobile-overview', true)
  await cdp.send('Emulation.setDeviceMetricsOverride', { width: 320, height: 740, deviceScaleFactor: 1, mobile: true })
  await frame()
  await captureDensity('web-permissions-mobile-320', true)
  await cdp.send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true })
  await frame()
  await cdp.click('[data-permission-section="filesystem"] h2 a[href="#permission-filesystem"]')
  await wait(
    () => cdp.evaluate(`document.activeElement?.dataset.permissionSection==='filesystem'`),
    'mobile file section focus',
  )
  check(await noOverflow(), '390px Chinese access page has no horizontal overflow')
  await screenshot('web-permissions-mobile-files')
  await select('[data-permission-default="path.workspace"] button[aria-haspopup="dialog"]', '只读')
  await cdp.click('[data-permission-section="network"] h2 a[href="#permission-network"]')
  await wait(
    () => cdp.evaluate(`document.activeElement?.dataset.permissionSection==='network'`),
    'mobile network section focus',
  )
  check(await noOverflow(), 'mobile network section remains within the viewport')
  await screenshot('web-permissions-mobile-network')
  await cdp.click('[data-permission-section="tools"] h2 a[href="#permission-tools"]')
  await wait(
    () => cdp.evaluate(`document.activeElement?.dataset.permissionSection==='tools'`),
    'mobile tool section focus',
  )
  check(await noOverflow(), 'mobile tool section and remembered rules remain within the viewport')
  await screenshot('web-permissions-mobile-tools')
  await select('[data-permission-default="tools.default"] button[aria-haspopup="dialog"]', '拒绝')
  await captureDensity('web-permissions-mobile-unsaved', true)
  await save('保存权限')
  check(
    fixture.values.global.path.workspace === 'readonly' && fixture.values.global.tools.default === 'deny',
    'mobile compact fields and toolbar save real file and tool edits together',
  )
  await cdp.click('[data-permission-rule-link="command"]')
  await wait(
    () => cdp.evaluate(`document.activeElement?.dataset.permissionRuleTable==='command'`),
    'mobile command rule group focus',
  )
  await frame()
  check(
    (await noOverflow()) &&
      (await cdp.evaluate(
        `document.querySelector('[data-permission-rule-table="command"] [data-permission-rule-heading]').getBoundingClientRect().top >= document.querySelector('[data-permission-toolbar]').getBoundingClientRect().bottom && [...document.querySelectorAll('[data-permission-rule-table="command"] [data-permission-rule]')].every(row=>row.dataset.permissionRule==='command')`,
      )),
    'mobile group navigation keeps the heading visible and its rules strictly separated',
  )
  await screenshot('web-permissions-mobile-command-rules')
  const mobileOriginalSession = clone(fixture.values.session)
  await select('[data-permission-scope] button[aria-haspopup="dialog"]', '当前会话权限')
  await wait(loaded, 'mobile current-session scope')
  check(
    (await defaultChoicesMatch({
      'path.workspace': '只读',
      'path.external': '只读',
      'tools.default': '拒绝',
      'tools.read-only': '允许',
    })) && (await defaultSourcesMatch('inherit')) && (await compactSourcesMatch('继承', '本层', '上层配置')),
    'mobile session defaults show the short Chinese source choices and distinct values',
  )
  await captureDensity('web-permissions-mobile-session-inherited', true)
  await chooseSession('Other project conversation')
  await waitSession(8)
  await openSessionPicker()
  await cdp.fill(pickerSearch, '')
  await wait(() => visible('[data-option-menu-item]'), 'mobile shared picker displays conversation options')
  await settleSessionPicker()
  await screenshot('web-permissions-mobile-session-picker')
  await cdp.clickText('[data-option-menu-item] .font-medium', 'Other project conversation', true)
  await select('[data-permission-default="tools.default"] button[aria-haspopup="dialog"]', '拒绝')
  await save('保存权限')
  check(
    fixture.writes.at(-1).sessionId === 8 &&
      fixture.sessionValues[8].tools.default === 'deny' &&
      JSON.stringify(fixture.values.session) === JSON.stringify(mobileOriginalSession) &&
      (await noOverflow()),
    'mobile shared dropdown selects a different session and saves only that session permission',
  )

  const approvals = '[data-saved-rules]'
  const advanced = '[data-permission-advanced]'
  const savedRow = approvals + ' [data-saved-rule-id]'
  const searchApprovals = approvals + ' input[aria-label="搜索已保存的审批规则"]'
  const createApproval = approvals + ' button[aria-label="创建工具规则"]'
  const scrollTo = async (selector) => {
    await cdp.evaluate(`document.querySelector(${JSON.stringify(selector)}).scrollIntoView({block:'center',behavior:'instant'})`)
    await frame()
  }
  const checkSecondaryLayout = async (wide) => {
    check(
      (await noOverflow()) &&
        (await cdp.evaluate(`(() => {
          const approvals = document.querySelector('${approvals}');
          const saved = approvals.getBoundingClientRect();
          const advanced = document.querySelector('${advanced}').getBoundingClientRect();
          const columns = approvals.querySelector('.saved-rules-column');
          const rows = [...approvals.querySelectorAll('[data-saved-rule-id]')];
          const wide = ${wide};
          return Math.abs(saved.left - advanced.left) <= 1 && Math.abs(saved.right - advanced.right) <= 1 &&
            advanced.top > saved.bottom && rows.length === 3 && Boolean(columns.getClientRects().length) === wide &&
            rows.every(row => {
              const target = row.querySelector('.saved-rule-target').getBoundingClientRect();
              const mode = row.querySelector('.saved-rule-mode').getBoundingClientRect();
              const action = row.querySelector('.saved-rule-remove').getBoundingClientRect();
              return wide ? mode.left >= target.right && action.left >= mode.right : mode.top >= target.bottom;
            }) && [...approvals.querySelectorAll('button,input,code')].every(control => {
              const box = control.getBoundingClientRect();
              return !box.width || (box.left >= saved.left && box.right <= saved.right);
            });
        })()`)),
      wide ? 'desktop approvals align into columns above full-width JSON' : 'phone approvals stack details without overflow',
    )
  }

  await navigate('/settings/permissions?view=access&scope=global')
  await wait(loaded, 'global permissions for the secondary sections')
  await wait(
    () => cdp.evaluate(`(() => {
      const approvals = document.querySelector('${approvals}');
      return approvals?.innerText.includes('暂无审批规则。');
    })()`),
    'empty saved approvals finish loading',
  )
  check(
    (await noOverflow()) && (await cdp.evaluate(`!document.querySelector('[data-saved-rules-controls]') && !document.querySelector('${advanced}').open`)),
    'empty approvals omit filters and advanced JSON starts collapsed',
  )
  await cdp.evaluate('window.__botProbe.dismissToasts()')
  await scrollTo(approvals)
  await screenshot('web-permissions-approvals-empty-mobile')
  await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: false })
  await cdp.send('Emulation.setDeviceMetricsOverride', { width: 1920, height: 1080, deviceScaleFactor: 1, mobile: false })

  const globalApproval = { scope: 'global', source: 'user', created_at: '2026-01-01', updated_at: '2026-01-01' }
  fixture.rules.push(
    { ...globalApproval, id: 810, subject_kind: 'tool', tool_name: 'agena.shell.exec', qualifier: 'git status', mode: 'ask' },
    {
      ...globalApproval,
      id: 811,
      subject_kind: 'network_access',
      network_target: '[2001:db8:abcd:1234:abcd:1234:abcd:1234]:443',
      mode: 'deny',
      reason: '用于验证长目标与审批备注的自动换行。',
    },
    { ...globalApproval, id: 809, subject_kind: 'tool', tool_name: 'agena.fs.read', mode: 'allow', revoked_at: '2026-01-02' },
  )
  await cdp.click(approvals + ' button[aria-label="刷新权限规则"]')
  await wait(() => cdp.evaluate(`document.querySelectorAll('${savedRow}').length === 3`), 'three saved approval examples load')
  await checkSecondaryLayout(true)
  await scrollTo(approvals)
  await screenshot('web-permissions-approvals-desktop')

  await select(approvals + ' button[aria-label^="规则状态"]', '已撤销')
  check(
    await cdp.evaluate(`document.querySelectorAll('${savedRow}').length === 1 && !!document.querySelector('[data-saved-rule-id="809"]')`),
    'the approval status filter keeps the revoked rule',
  )
  await cdp.fill(searchApprovals, 'missing rule')
  await wait(() => cdp.evaluate(`document.querySelector('${approvals}').innerText.includes('没有匹配的规则。')`), 'filter results show a distinct empty state')
  await cdp.clickText(approvals + ' button', '清除筛选', true)
  await wait(() => cdp.evaluate(`document.querySelectorAll('${savedRow}').length === 3`), 'clearing filters restores all saved rules')
  await cdp.click(approvals + ' input[aria-label="全选权限规则"]')
  check(
    await cdp.evaluate(`document.querySelectorAll('${savedRow} input:checked').length === 3 && !!document.querySelector('.saved-rules-bulk')`),
    'select all from the results header exposes bulk actions for the visible rules',
  )
  await cdp.send('Emulation.setDeviceMetricsOverride', { width: 320, height: 740, deviceScaleFactor: 1, mobile: true })
  await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 1 })
  await frame()
  check(await bulkFits(approvals + ' .saved-rules-bulk', true), 'saved-rule bulk actions stack at 320 px without overflow')
  await cdp.click(approvals + ' button[aria-label="全不选"]')
  await scrollTo(approvals)
  await checkSecondaryLayout(false)
  await cdp.send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true })
  await scrollTo(approvals)
  await screenshot('web-permissions-approvals-mobile')
  await cdp.send('Emulation.setDeviceMetricsOverride', { width: 320, height: 740, deviceScaleFactor: 1, mobile: true })
  await cdp.click(createApproval)
  check(
    await cdp.evaluate(`(() => {
      const trigger = document.querySelector('${createApproval}');
      const form = document.getElementById(trigger.getAttribute('aria-controls'));
      const filters = document.querySelector('${approvals} [data-saved-rules-controls]');
      return trigger.getAttribute('aria-expanded') === 'true' && form.contains(document.activeElement) &&
        form.getBoundingClientRect().bottom < filters.getBoundingClientRect().top;
    })()`),
    'the mobile create button focuses its form immediately above the filters',
  )
  await cdp.clickText(approvals + ' form button', '取消', true)
  await cdp.click(advanced + ' summary')
  check((await noOverflow()) && (await visible(advanced + ' textarea')), 'the advanced JSON editor fits a 320px viewport')
  await scrollTo(advanced + ' textarea')
  await screenshot('web-permissions-advanced-mobile')

  await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: false })
  await cdp.send('Emulation.setDeviceMetricsOverride', { width: 1920, height: 1080, deviceScaleFactor: 1, mobile: false })
  await frame()
  const writesBeforeJson = fixture.writes.length
  await cdp.fill(advanced + ' textarea', '[]')
  await cdp.clickText(advanced + ' button', '应用 JSON', true)
  check(
    (await cdp.evaluate(`(() => {
      const field = document.querySelector('${advanced} textarea');
      const error = document.getElementById(field.getAttribute('aria-describedby'));
      return field.getAttribute('aria-invalid') === 'true' && error?.getAttribute('role') === 'alert' &&
        error.getBoundingClientRect().top >= field.getBoundingClientRect().bottom;
    })()`)) && fixture.writes.length === writesBeforeJson,
    'advanced JSON shows its validation error below the editor without persisting an invalid draft',
  )
  const validJson = clone(fixture.values.global)
  validJson.tools.default = 'ask'
  await cdp.fill(advanced + ' textarea', JSON.stringify(validJson, null, 2))
  await cdp.clickText(advanced + ' button', '应用 JSON', true)
  check(
    (await visible('[data-permission-savebar]')) && fixture.writes.length === writesBeforeJson &&
      (await cdp.evaluate(`!document.querySelector('${advanced} [role="alert"]')`)),
    'applying valid JSON updates the draft while Save permission owns persistence',
  )
  await save('保存权限')
  check(fixture.values.global.tools.default === 'ask', 'saving the global draft persists the parsed advanced JSON')
  await cdp.evaluate('window.__botProbe.dismissToasts()')
  await scrollTo(advanced)
  await screenshot('web-permissions-advanced-desktop')

  await exerciseMobileWorkspaceSelection()
}

module.exports = { createFixture, run }
