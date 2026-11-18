// Real SettingsPage, single permission editor and native pointer/keyboard input.
// BOT_PROBE_OFFLINE=1 PERMISSIONS_PROBE_ONLY=1 BOT_EVIDENCE=/tmp/agena-permissions node scripts/probe-bot.cjs
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')

const clone = (value) => JSON.parse(JSON.stringify(value))
const defaults = {
  path: { workspace: 'readwrite', external: 'deny', rules: { '<state>': 'readwrite' } },
  network: { default: 'allow', inbound: 'deny', hostLoopback: 'deny' },
  tools: {
    default: 'allow',
    names: { 'agena.interaction.ask': 'allow', 'agena.web.search': 'allow', 'agena.web.fetch': 'allow' },
    rules: {
      '*': { 'read-only': 'allow' },
      ...Object.fromEntries(
        ['exec', 'spawn', 'watch', 'open'].map((name) => [
          'agena.shell.' + name,
          { 'no-op': 'allow', routine: 'allow', dangerous: 'deny' },
        ]),
      ),
      'agena.shell.write': { dangerous: 'deny' },
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
      } else result[key] = clone(value)
    }
    return result
  }, {})
}
function createFixture({ execution }) {
  const values = {
    global: {
      path: { rules: { '<workspace>/docs': 'readonly' } },
      network: {
        default: 'deny',
        inbound: 'deny',
        hostLoopback: 'deny',
        allow: [{ to: '198.51.100.24', protocol: 'tcp', ports: [443] }],
        deny: [{ to: '192.0.2.13' }],
      },
      tools: {
        names: { 'agena.fs.read': 'allow', 'agena.web.fetch': 'ask' },
        rules: { 'agena.shell.exec': { 'git status': 'allow', 'git push *': 'ask' } },
      },
    },
    workspace: {},
    session: {},
  }
  const writes = [],
    ruleWrites = [],
    catalogRequests = [],
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
    catalogRequests,
    sessionReads,
    async handle(req, url, json) {
      const p = url.pathname
      const layer = p.match(/^\/api\/v1\/settings\/layers\/(global|workspace)$/)?.[1]
      if (layer && url.searchParams.get('path') === 'permission' && req.method === 'GET') {
        json({
          value: clone(values[layer]),
          inherited_value: inherited(layer),
          config_path: layer === 'workspace' ? '/fixture/project/.agena/config.json' : '/fixture/global/config.json',
          config_found: true,
        })
      } else if (layer && req.method === 'PUT') {
        const update = await body(req)
        assert.equal(update.path, 'permission')
        values[layer] = clone(update.value)
        writes.push({ scope: layer, ...update })
        json({ ok: true, value: clone(update.value), reloaded: true })
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
        json({ items: clone(rules), page: { has_more: false } })
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
        json({
          items: [
            { id: 7, path: '/fixture/project' },
            { id: 8, path: '/fixture/other-project' },
          ],
          page: { has_more: false },
        })
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
      `document.querySelectorAll('[data-permission-section]').length===3 && document.querySelector('.permission-studio')?.getAttribute('aria-busy')==='false'`,
    )
  const sessionTrigger = '[data-permission-session-picker] button[aria-haspopup="dialog"]'
  const pickerSearch = '[data-keyboard-overlay="true"] input'
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
  const save = async (label = 'Save permission') => {
    await cdp.clickText('[data-permission-savebar] button', label)
    await wait(async () => !(await visible('[data-permission-savebar]')), 'combined permission save completes')
  }
  const noOverflow = () =>
    cdp.evaluate(
      `document.documentElement.scrollWidth<=innerWidth+1 && [...document.querySelectorAll('.permission-studio,[data-permission-section],[data-permission-savebar]')].every(el=>el.getBoundingClientRect().right<=innerWidth+1)`,
    )
  const densityMeasurements = []
  const pageTop = async () => {
    await cdp.evaluate(
      `(() => { for(let element=document.querySelector('.permission-studio');element;element=element.parentElement) { if(element.scrollHeight>element.clientHeight && /auto|scroll/.test(getComputedStyle(element).overflowY)) element.scrollTo({top:0,behavior:'instant'}); } document.scrollingElement?.scrollTo({top:0,behavior:'instant'}); })()`,
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
        return {key:element.dataset.permissionDefault||element.dataset.permissionRule||element.dataset.permissionSummary||'',text:element.innerText.trim().replace(/\\s+/g,' '),x:Math.round(rect.x),y:Math.round(rect.y),width:Math.round(rect.width),height:Math.round(rect.height),bottom:Math.round(rect.bottom),fontSize:style.fontSize,visible:rect.width>0&&rect.height>0&&rect.top>=0&&rect.bottom<=innerHeight-48};
      };
      const ruleKinds={filesystem:['filesystem','path'],network:['network'],tools:['tool-name','command']};
      const rules=[...document.querySelectorAll('[data-permission-rule]')];
      const domains=[...document.querySelectorAll('[data-permission-section]')].map(element=>({id:element.dataset.permissionSection,...describe(element),defaults:[...element.querySelectorAll('[data-permission-default]')].map(describe),rules:rules.filter(row=>ruleKinds[element.dataset.permissionSection]?.includes(row.dataset.permissionRule)).map(describe)}));
      const ruleGroups=[...document.querySelectorAll('[data-permission-rule-table]')].map(group=>({kind:group.dataset.permissionRuleTable,heading:group.querySelector('h2')?.innerText.trim(),...describe(group),rows:[...group.querySelectorAll('[data-permission-rule]')].map(row=>({kind:row.dataset.permissionRule,...describe(row)}))}));
      const clippedModes=[...document.querySelectorAll('[data-permission-default]:not([data-permission-default="network.source"]) button[aria-haspopup="dialog"] .truncate,[data-permission-mode] .truncate')].filter(element=>element.getBoundingClientRect().height>0&&element.scrollWidth>element.clientWidth+1).map(element=>element.innerText);
      return {viewport:{width:innerWidth,height:innerHeight},dark:document.documentElement.classList.contains('dark'),toolbar:document.querySelector('[data-permission-toolbar]')?describe(document.querySelector('[data-permission-toolbar]')):null,domains,ruleGroups,summaries:[...document.querySelectorAll('[data-permission-summary]')].map(describe),clippedModes,horizontalOverflow:document.documentElement.scrollWidth>innerWidth+1};
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
    check(
      measurement.ruleGroups.length === 4 &&
        measurement.ruleGroups.every(
          (group) =>
            group.heading && group.rows.every((row) => row.kind === group.kind && row.width > 0 && row.height > 0),
        ),
      name + ': all four rule groups are separate and existing rules remain expanded',
    )
    if (mobile) {
      check(
        measurement.summaries.length === 3 && measurement.summaries.every((item) => item.visible),
        name + ': all three policy summaries are visible without scrolling',
      )
    } else {
      const minimumDefaults = { filesystem: 2, network: 3, tools: 2 }
      check(
        measurement.domains.length === 3 &&
          measurement.domains.every(
            (domain) =>
              domain.defaults.length >= minimumDefaults[domain.id] && domain.defaults.every((item) => item.visible),
          ),
        name + ': actual file, network, and tool defaults fit in the first viewport',
      )
      check(
        measurement.ruleGroups.some((group) => group.rows.some((row) => row.visible)),
        name + ': compact defaults leave room for rule rows in the first viewport',
      )
      check(
        measurement.toolbar && measurement.toolbar.height <= 100,
        name + ': scope, target, and save controls use one compact toolbar',
      )
    }
  }
  await cdp.send('Page.navigate', { url: base + '/chat?sessionId=7' })
  await wait(() => cdp.evaluate(`window.__botProbe?.activeComposerSession()==='7'`), 'current project conversation')
  await cdp.evaluate(`window.__botProbe.setAppLocale('en-US')`)
  await cdp.evaluate(`window.__botProbe.applyAppearanceSettingsToDom({useSystemTheme:false,themeVariant:'dark'})`)
  await navigate('/settings/permissions?view=filesystem&scope=global&sessionId=999')
  await wait(loaded, 'all permission sections on a single page')
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
      `document.querySelectorAll('.permission-studio').length===1 && [...document.querySelectorAll('[data-permission-section]')].every(section=>section.querySelector('h2 a[href="#permission-'+section.dataset.permissionSection+'"]'))`,
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
    JSON.stringify(await groupCounts()) === JSON.stringify({ filesystem: 2, network: 2, 'tool-name': 4, command: 12 }),
    'each rule type retains its complete local and inherited rules',
  )
  check(
    await cdp.evaluate(
      `!document.querySelector('[data-permission-rule-table="filesystem"] .rule-table-condition, [data-permission-rule-table="tool-name"] .rule-table-condition, .rule-table-type') && !!document.querySelector('[data-permission-rule-table="network"] .rule-table-condition') && !!document.querySelector('[data-permission-rule-table="command"] .rule-table-condition')`,
    ),
    'each table shows only columns meaningful for its rule type',
  )
  await cdp.fill(`${ruleGroup('filesystem')} input[aria-label$="Search rules…"]`, 'docs')
  await frame()
  check(
    JSON.stringify(await groupCounts()) === JSON.stringify({ filesystem: 1, network: 2, 'tool-name': 4, command: 12 }),
    'file search only filters the file rules group',
  )
  await cdp.fill(`${ruleGroup('tool-name')} input[aria-label$="Search rules…"]`, 'fetch')
  await frame()
  check(
    JSON.stringify(await groupCounts()) === JSON.stringify({ filesystem: 1, network: 2, 'tool-name': 1, command: 12 }),
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
      `document.querySelector('[data-permission-rule-table="command"] h2').getBoundingClientRect().top >= document.querySelector('[data-permission-toolbar]').getBoundingClientRect().bottom && document.querySelectorAll('[data-permission-rule-table]').length===4`,
    ),
    'rule count links navigate to a visible group heading without filtering away other types',
  )
  await screenshot('web-permissions-command-rules')

  await select('[data-permission-default="path.workspace"] button[aria-haspopup="dialog"]', 'Read only')
  await select(`${section('tools')} button[aria-label^="Default tool mode:"]`, 'Deny')
  await select(
    '[data-permission-default="tools.read-only"] button[aria-haspopup="dialog"]',
    'Deny · Follow tool default',
  )
  check(
    await cdp.evaluate(
      `document.querySelector('[data-permission-default="tools.read-only"] button[aria-haspopup="dialog"]')?.innerText.trim()==='Deny'`,
    ),
    'read-only Auto displays the actual tool default',
  )
  await select(
    `${rule('filesystem', '<workspace>/docs')} [data-permission-mode] button[aria-haspopup="dialog"]`,
    'Deny',
  )
  await select(`${rule('tool-name', 'agena.web.fetch')} [data-permission-mode] button[aria-haspopup="dialog"]`, 'Allow')
  await select(
    `${rule('command', 'agena.shell.exec', 'git status')} [data-permission-mode] button[aria-haspopup="dialog"]`,
    'Deny',
  )
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
      `![...document.querySelectorAll('[data-permission-rule][data-rule-scope="inherited"] button[aria-haspopup="dialog"],[data-permission-rule][data-rule-scope="inherited"] [data-permission-row-action]')].some(control=>!control.disabled&&!control.readOnly)`,
    ),
    'inherited rule rows have no misleading mutation controls',
  )
  check(fixture.writes.length === 0, 'file, network, and tool edits share an unsaved draft')
  await cdp.click(`${section('network')} h2 a[href="#permission-network"]`)
  check(!(await visible('[role="dialog"]')), 'in-page section navigation keeps the draft without a discard dialog')
  for (const destination of [
    'view=access&scope=workspace',
    'view=overview&scope=global',
    'view=sandbox&scope=global',
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
      fixture.values.global.tools.rules['agena.shell.exec']['git status'] === 'deny' &&
      fixture.values.global.tools.rules['agena.shell.exec']['git push *'] === 'ask' &&
      Object.keys(fixture.values.global.tools.rules['agena.shell.exec']).join('|') === 'git status|git push *' &&
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
  await navigate('/settings/permissions?view=access&scope=workspace')
  await wait(loaded, 'workspace access scope')
  check(
    await cdp.evaluate(
      `document.querySelector('[data-permission-target]')?.getAttribute('title')==='/fixture/project/.agena/config.json' && !document.querySelector('[data-permission-target] button')`,
    ),
    'workspace target names the server configuration file without a misleading project picker',
  )
  await select(`${section('tools')} button[aria-label^="Default tool mode:"]`, 'Allow')
  await save()
  check(
    fixture.values.workspace.tools.default === 'allow' && fixture.values.global.tools.default === 'deny',
    'workspace saves stay isolated from global permission changes',
  )

  await navigate('/settings/permissions?view=tools&scope=session&sessionId=999')
  await wait(loaded, 'session access scope')
  await select('[data-permission-default="path.workspace"] button[aria-haspopup="dialog"]', 'Read only')
  await cdp.clickText('.permission-presets button', 'Block external network')
  await select(`${section('tools')} button[aria-label^="Default tool mode:"]`, 'Auto')
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
  await select('[data-permission-default="tools.default"] button[aria-haspopup="dialog"]', 'Deny')
  await save()
  check(
    fixture.writes.at(-1).sessionId === 164 && fixture.sessionValues[164].tools.default === 'deny',
    'an older selected session is editable even when it is absent from the recent list',
  )
  await chooseSession('Atlas')
  await waitSession(42)
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
  }

  await navigate('/settings/permissions?view=access&scope=global')
  await wait(loaded, 'global scope for mobile')
  await cdp.send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true })
  await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 1 })
  await cdp.evaluate(`window.__botProbe.setAppLocale('zh-CN')`)
  await frame()
  await captureDensity('web-permissions-mobile-overview', true)
  await cdp.click('[data-permission-sections] a[href="#permission-filesystem"]')
  await wait(
    () => cdp.evaluate(`document.activeElement?.dataset.permissionSection==='filesystem'`),
    'mobile file section focus',
  )
  check(await noOverflow(), '390px Chinese access page has no horizontal overflow')
  await screenshot('web-permissions-mobile-files')
  await select('[data-permission-default="path.workspace"] button[aria-haspopup="dialog"]', '只读')
  await cdp.click('[data-permission-sections] a[href="#permission-network"]')
  await wait(
    () => cdp.evaluate(`document.activeElement?.dataset.permissionSection==='network'`),
    'mobile network section focus',
  )
  check(await noOverflow(), 'mobile network section remains within the viewport')
  await screenshot('web-permissions-mobile-network')
  await cdp.click('[data-permission-sections] a[href="#permission-tools"]')
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
        `document.querySelector('[data-permission-rule-table="command"] h2').getBoundingClientRect().top >= document.querySelector('[data-permission-toolbar]').getBoundingClientRect().bottom && [...document.querySelectorAll('[data-permission-rule-table="command"] [data-permission-rule]')].every(row=>row.dataset.permissionRule==='command')`,
      )),
    'mobile group navigation keeps the heading visible and its rules strictly separated',
  )
  await screenshot('web-permissions-mobile-command-rules')
  const mobileOriginalSession = clone(fixture.values.session)
  await select('[data-permission-scope] button[aria-haspopup="dialog"]', '当前会话权限')
  await wait(loaded, 'mobile current-session scope')
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
}

module.exports = { createFixture, run }
