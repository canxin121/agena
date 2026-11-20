// Real composer/profile settings UI against an isolated in-memory API.
// Reuses the offline Chrome pipe + Vite harness; run: node scripts/probe-agent-profiles.cjs
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const os = require('node:os')
const path = require('node:path')
const { spawn } = require('node:child_process')
const offline = require('./bot-probe-offline.cjs')

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
async function wait(check, description, ms = 45000) {
  const until = Date.now() + ms
  let last
  while (Date.now() < until) {
    try {
      const value = await check()
      if (value) return value
    } catch (error) {
      last = error
    }
    await sleep(100)
  }
  throw new Error(`Timed out: ${description}${last ? ` (${last})` : ''}`)
}

class Cdp {
  constructor(socket) {
    this.socket = socket
    this.sequence = 0
    this.pending = new Map()
    this.events = new Map()
    socket.addEventListener('message', (event) => {
      const message = JSON.parse(String(event.data))
      if (message.id) {
        const pending = this.pending.get(message.id)
        if (!pending) return
        clearTimeout(pending.timer)
        this.pending.delete(message.id)
        if (message.error) pending.reject(new Error(JSON.stringify(message.error)))
        else pending.resolve(message.result)
      } else this.events.get(message.method)?.(message.params)
    })
  }
  send(method, params = {}) {
    const id = ++this.sequence
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`CDP timeout: ${method}`))
      }, 30000)
      this.pending.set(id, { resolve, reject, timer })
      this.socket.send(JSON.stringify({ id, method, params }))
    })
  }
  async evaluate(expression) {
    const reply = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true })
    if (reply.exceptionDetails)
      throw new Error(reply.exceptionDetails.exception?.description || JSON.stringify(reply.exceptionDetails))
    return reply.result?.value
  }
  async click(selector) {
    const point = await wait(
      () =>
        this.evaluate(`(() => {
      const node = [...document.querySelectorAll(${JSON.stringify(selector)})].find(el => el.getClientRects().length && getComputedStyle(el).visibility !== 'hidden');
      if (!node || node.disabled || !node.getClientRects().length) return null;
      node.scrollIntoView({block:'center',behavior:'instant'}); const r = node.getBoundingClientRect();
      const point={x:r.x+r.width/2,y:r.y+r.height/2};
      const hit=document.elementFromPoint(point.x,point.y);
      return hit && node.contains(hit) ? {...point,touch:matchMedia('(pointer:coarse)').matches} : null; })()`),
      selector,
    )
    // A touch-emulated viewport must receive real taps: mouse events do not
    // exercise the mobile keyboard-dismiss/click path and can miss the action.
    if (point.touch) {
      await this.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: point.x, y: point.y }] })
      await this.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
    } else {
      const coords = { x: point.x, y: point.y }
      await this.send('Input.dispatchMouseEvent', { type: 'mousePressed', ...coords, button: 'left', clickCount: 1 })
      await this.send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...coords, button: 'left', clickCount: 1 })
    }
  }
  async clickText(selector, label, exact = false) {
    const point = await wait(
      () =>
        this.evaluate(`(() => {
      const n=[...document.querySelectorAll(${JSON.stringify(selector)})].find(el=>el.getClientRects().length && !el.disabled && (${JSON.stringify(exact)} ? el.textContent.trim()===${JSON.stringify(label)} : el.textContent.includes(${JSON.stringify(label)})));
      if(!n) return null; n.scrollIntoView({block:'center'}); const r=n.getBoundingClientRect();
      const x=r.x+r.width/2,y=r.y+r.height/2; return n.contains(document.elementFromPoint(x,y)) ? {x,y,touch:matchMedia('(pointer:coarse)').matches} : null;
    })()`),
      `visible ${selector} containing ${label}`,
    )
    if (point.touch) {
      await this.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: point.x, y: point.y }] })
      await this.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
    } else {
      for (const type of ['mousePressed', 'mouseReleased'])
        await this.send('Input.dispatchMouseEvent', { type, x: point.x, y: point.y, button: 'left', clickCount: 1 })
    }
  }
  async fill(selector, text) {
    await this.click(selector)
    await this.send('Input.dispatchKeyEvent', {
      type: 'keyDown',
      key: 'a',
      code: 'KeyA',
      windowsVirtualKeyCode: 65,
      modifiers: 2,
    })
    await this.send('Input.dispatchKeyEvent', {
      type: 'keyUp',
      key: 'a',
      code: 'KeyA',
      windowsVirtualKeyCode: 65,
      modifiers: 2,
    })
    await this.send('Input.insertText', { text })
    await this.evaluate('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))')
  }
  async key(key, code = key, number = 0) {
    await this.send('Input.dispatchKeyEvent', {
      type: 'keyDown',
      key,
      code,
      windowsVirtualKeyCode: number,
      ...(key === 'Enter' ? { text: '\r' } : {}),
    })
    await this.evaluate('new Promise(resolve => requestAnimationFrame(resolve))')
    await this.send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode: number })
    await this.evaluate('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))')
  }
  async screenshot(file) {
    const { data } = await this.send('Page.captureScreenshot', { format: 'png' })
    await fs.writeFile(file, Buffer.from(data, 'base64'))
  }
  close() {
    for (const pending of this.pending.values()) clearTimeout(pending.timer)
    this.socket.close()
  }
}

const composerInspection = `(() => ({url:location.href,menus:[...document.querySelectorAll('[data-keyboard-overlay="true"]')].map(node=>({visible:!!node.getClientRects().length,text:node.innerText})),composers:[...document.querySelectorAll('[data-testid="composer-profile-trigger"]')].map(node=>{let owner=node.__vueParentComponent;while(owner&&!owner.type.__file?.endsWith('/pages/ChatPage.vue')&&owner.type.__name!=='ChatPage')owner=owner.parent;const state=owner?.setupState;const rect=node.getBoundingClientRect();return {visible:!!node.getClientRects().length&&getComputedStyle(node).visibility!=='hidden',hit:node.contains(document.elementFromPoint(rect.x+rect.width/2,rect.y+rect.height/2)),expanded:node.getAttribute('aria-expanded'),session:state?.chat.selectedSessionId,loadedSession:state?.chat.selectedSession?.id,profile:state?.chat.selectedSessionExecution?.profile,directory:state?.directoryStore?.currentDirectory,focused:state?.isFocusedWorkspacePane,open:state?.composerPickerOpen}})}))()`

const copy = (value) => JSON.parse(JSON.stringify(value ?? null))
const own = (value, key) => Object.prototype.hasOwnProperty.call(value, key)

function createFixture() {
  const builtins = {
    main: { title: 'Main', description: 'Coordinate general tasks and delegate focused work.', mode: 'primary' },
    explore: { title: 'Explore', description: 'Read the project and report focused evidence.', mode: 'subagent' },
    worker: { title: 'Worker', description: 'Implement a bounded task and verify the result.', mode: 'subagent' },
    reviewer: { title: 'Reviewer', description: 'Review correctness and report actionable findings.', mode: 'all' },
  }
  const defaults = {
    builtin_profiles: builtins,
    profiles: {},
    default_profile: 'main',
    default_subagent_profile: 'worker',
    prompts: {},
    ui: {},
  }
  const values = { global: {}, workspace: {} }
  const selected = { 7: null, 42: null }
  const versions = { 7: 1, 42: 1 }
  const writes = [],
    requests = []
  const read = (layer, key) => ({
    value: own(values[layer], key) ? copy(values[layer][key]) : null,
    source: 'file',
    path: key,
    config_path: '/fixture/' + layer + '/config.json',
    config_found: true,
    has_override: own(values[layer], key),
    ...(own(values[layer], key) ? { override_path: key } : {}),
    inherited_value: copy(layer === 'global' ? defaults[key] : (values.global[key] ?? defaults[key])),
  })
  const effective = (key) => copy(values.workspace[key] ?? values.global[key] ?? defaults[key])
  const bundle = (key) => ({
    effective: { source: 'effective', value: effective(key), path: key },
    file: read('global', key),
    global: read('global', key),
    workspace: read('workspace', key),
  })
  const selection = { provider: 'fixture', adapter: 'local', model: 'fixture-model' }
  const context = (id) => ({
    profile: selected[id] || 'main',
    profile_is_default: !selected[id],
    global: id === 42,
    effective_workspace_root: id === 42 ? null : '/fixture/project',
    model_provider: 'fixture',
    model_adapter: 'local',
    model_id: 'fixture-model',
    selection,
    inherited_selection: selection,
    selected_selection: {},
    effective_permission: {},
    permission: {},
  })
  const session = (id) => ({
    id,
    title: id === 42 ? 'Global profile fixture' : 'Project profile fixture',
    workspace_id: id === 42 ? null : 7,
    owner_bot_id: null,
    owner_surface: null,
    depth: 0,
    root_id: id,
    parent_id: null,
    relation_kind: 'root',
    lifecycle_state: 'ready',
    is_subagent: false,
    message_count: 0,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-02T00:00:00Z',
    version: versions[id],
    state: { kind: 'ready', data: {} },
    execution: context(id),
  })
  const state = (id) => ({
    session: session(id),
    execution: context(id),
    parts: [],
    part_page: null,
    latest_event_seq: 1,
    queued_messages: [],
    queue_paused: false,
    background_activities: [],
  })
  const body = async (req) => {
    let text = ''
    for await (const chunk of req) text += chunk
    return JSON.parse(text || '{}')
  }
  return {
    values,
    writes,
    requests,
    async middleware(req, res) {
      const url = new URL(req.url, 'https://agena-offline.invalid'),
        p = url.pathname
      requests.push({ method: req.method, path: p, query: url.search })
      const json = (value, status = 200) => {
        res.statusCode = status
        res.setHeader('Content-Type', 'application/json')
        res.end(JSON.stringify(value))
      }
      const layer = p.match(/^\/api\/v1\/settings\/layers\/(global|workspace)$/)?.[1]
      if (p === '/api/v1/settings/sources')
        return json(
          Object.fromEntries(JSON.parse(url.searchParams.get('paths') || '[]').map((key) => [key, bundle(key)])),
        )
      if (layer && req.method === 'GET') return json(read(layer, url.searchParams.get('path') || ''))
      if (layer && req.method === 'PUT') {
        const update = await body(req)
        values[layer][update.path] = copy(update.value)
        writes.push({ method: 'PUT', layer, ...update })
        return json({ changed: true, current: copy(update.value), after_revision: String(writes.length) })
      }
      if (layer && req.method === 'DELETE') {
        const key = url.searchParams.get('path')
        delete values[layer][key]
        writes.push({ method: 'DELETE', layer, path: key })
        return json({ changed: true, deleted: true })
      }
      if (p === '/api/v1/settings')
        return json(
          url.searchParams.get('source') === 'file'
            ? read('global', url.searchParams.get('path') || '')
            : { value: effective(url.searchParams.get('path') || '') || {} },
        )
      const sessionId = p.match(/^\/api\/v1\/sessions\/(\d+)$/)?.[1]
      if (sessionId && req.method === 'PUT') {
        const update = await body(req)
        assert(own(update, 'profile'), 'Only profile session updates are expected')
        selected[sessionId] = update.profile || null
        versions[sessionId]++
        writes.push({ method: 'PUT', layer: 'session', sessionId: Number(sessionId), ...update })
        return json(session(Number(sessionId)))
      }
      if (sessionId) return json(session(Number(sessionId)))
      if (/\/sessions\/\d+\/(state|messages)$/.test(p)) return json(state(Number(p.split('/')[4])))
      if (/\/sessions\/\d+\/(parts|runs)$/.test(p))
        return json({
          session_id: Number(p.split('/')[4]),
          parts: [],
          runs: [],
          user_message_count: 0,
          page: { has_more: false },
          version: 1,
        })
      if (p === '/api/v1/sessions')
        return json({ items: [session(7), session(42)], total: 2, page: { has_more: false } })
      if (p === '/auth/session') return json({ authenticated: true, disabled: true })
      if (p === '/api/v1/health') return json({ status: 'ok', database_connected: true })
      if (p === '/api/v1/runtime') return json({ default_selection: selection, workspace: null })
      if (p === '/api/v1/providers') return json([{ provider_id: 'fixture' }])
      if (p.endsWith('/configured-models'))
        return json([
          { adapter_id: 'local', enabled: true, models: [{ id: 'fixture-model', display_name: 'Fixture Model' }] },
        ])
      if (p === '/api/v1/plugins/surface') return json({ catalog: { commands: [] } })
      if (p === '/api/v1/workspaces/7') return json({ id: 7, path: '/fixture/project' })
      if (p === '/api/v1/workspaces')
        return json({ items: [{ id: 7, path: '/fixture/project' }], page: { has_more: false } })
      if (p === '/api/v1/workbench/fs/home') return json({ home: '/fixture' })
      if (p === '/api/v1/activities') return json({ items: [], activities: [] })
      if (p.endsWith('/events')) {
        res.setHeader('Content-Type', 'text/event-stream')
        return res.end(': keepalive\n\n')
      }
      if (req.method !== 'GET') return json({ error: 'Unexpected fixture mutation: ' + p }, 400)
      return json({ items: [], page: { has_more: false } })
    },
  }
}

async function main() {
  assert(!process.versions.bun, 'Run the offline browser probe with Node')
  const web = path.resolve(__dirname, '..')
  const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'agena-profile-probe-'))
  const chromeProfile = await fs.mkdtemp(path.join(os.tmpdir(), 'agena-profile-chrome-'))
  const evidence = process.env.AGENT_PROFILE_EVIDENCE || '/tmp/agena-agent-profiles'
  await fs.mkdir(evidence, { recursive: true })
  const fixture = createFixture(),
    errors = [],
    checks = [],
    layouts = []
  const check = (condition, label) => {
    assert(condition, label)
    checks.push(label)
    console.log('PASS ' + label)
  }
  let chrome, cdp
  try {
    await fs.writeFile(
      path.join(temporary, 'entry.ts'),
      `import '/src/main.ts'\nimport { router } from '/src/router.ts'\nimport { setAppLocale } from '/src/i18n/index.ts'\nsetAppLocale('en-US')\nwindow.__profileProbe={router,setAppLocale}\n`,
    )
    await offline.prepareOfflineConfig(web, temporary)
    const directory = await offline.buildOfflineApp(web, temporary)
    const base = 'https://agena-offline.invalid'
    chrome = spawn(
      process.env.CHROME_BIN || '/usr/bin/google-chrome',
      [
        '--headless=new',
        '--no-sandbox',
        '--disable-dev-shm-usage',
        '--disable-background-networking',
        '--disable-extensions',
        '--remote-debugging-pipe',
        '--disable-component-update',
        '--disable-sync',
        '--no-first-run',
        '--host-resolver-rules=MAP * ~NOTFOUND',
        '--user-data-dir=' + chromeProfile,
        'about:blank',
      ],
      { stdio: ['ignore', 'ignore', 'ignore', 'pipe', 'pipe'] },
    )
    const socket = new offline.PipeSocket(chrome)
    cdp = new Cdp(socket)
    const target = await cdp.send('Target.createTarget', { url: 'about:blank' })
    const attached = await cdp.send('Target.attachToTarget', { targetId: target.targetId, flatten: true })
    socket.sessionId = attached.sessionId
    cdp.events.set('Runtime.exceptionThrown', (value) =>
      errors.push(value.exceptionDetails.exception?.description || JSON.stringify(value)),
    )
    cdp.events.set(
      'Fetch.requestPaused',
      offline.offlineRequestHandler({ cdp, base, directory, middleware: fixture.middleware, errors }),
    )
    await cdp.send('Network.enable')
    await cdp.send('Network.emulateNetworkConditions', {
      offline: true,
      latency: 0,
      downloadThroughput: 0,
      uploadThroughput: 0,
    })
    await cdp.send('Runtime.enable')
    await cdp.send('Page.enable')
    await cdp.send('Fetch.enable', { patterns: [{ urlPattern: '*' }] })
    const trigger = '[data-testid="composer-profile-trigger"]'
    const defaults = '[data-testid="agent-profile-defaults"]'
    const popup = '[data-keyboard-overlay="true"]'
    const item = '[data-option-menu-item]'
    const visible = (selector) =>
      cdp.evaluate(
        `!![...document.querySelectorAll(${JSON.stringify(selector)})].find(node=>node.getClientRects().length&&getComputedStyle(node).visibility!=='hidden')`,
      )
    const live = (selector) =>
      `[...document.querySelectorAll(${JSON.stringify(selector)})].find(node=>node.getClientRects().length&&getComputedStyle(node).visibility!=='hidden')`
    const text = (selector) => cdp.evaluate(`(${live(selector)})?.textContent || ''`)
    const navigate = (url) => cdp.evaluate(`window.__profileProbe.router.push(${JSON.stringify(url)})`)
    const composerReady = async (variant) => {
      let previous = '',
        since = 0
      await wait(async () => {
        const state = await cdp.evaluate(composerInspection)
        const current = state.composers.find(
          (value) =>
            value.visible &&
            value.hit &&
            value.focused !== false &&
            String(value.session) === String(variant.session) &&
            String(value.loadedSession) === String(variant.session) &&
            value.profile &&
            (variant.scope !== 'workspace' || value.directory === '/fixture/project'),
        )
        if (!current) return false
        const key = JSON.stringify([state.url, current])
        if (key !== previous) {
          previous = key
          since = Date.now()
          return false
        }
        return Date.now() - since >= 300
      }, variant.name + ' settled active composer')
    }
    const chooseNative = async (selector, value) => {
      const index = await cdp.evaluate(
        `Array.from((${live(selector)}).options).findIndex(option=>option.value===${JSON.stringify(value)})`,
      )
      assert(index >= 0, 'Requested native option exists: ' + value)
      await cdp.click(selector)
      await cdp.key('Home', 'Home', 36)
      for (let i = 0; i < index; i++) await cdp.key('ArrowDown', 'ArrowDown', 40)
      await cdp.key('Enter', 'Enter', 13)
      await wait(
        () => cdp.evaluate(`(${live(selector)})?.value===${JSON.stringify(value)}`),
        'native selection ' + value,
      )
    }
    const layout = async (label, selector, fullHeight) => {
      const result = await cdp.evaluate(
        `(() => {const n=[...document.querySelectorAll(${JSON.stringify(selector)})].find(n=>n.getClientRects().length);if(!n)return null;const r=n.getBoundingClientRect();return {left:r.left,right:r.right,top:r.top,bottom:r.bottom,width:innerWidth,height:innerHeight,scrollWidth:document.documentElement.scrollWidth,clientWidth:document.documentElement.clientWidth}})()`,
      )
      layouts.push({ label, ...result })
      check(
        result &&
          result.left >= -1 &&
          result.right <= result.width + 1 &&
          result.scrollWidth <= result.clientWidth + 1 &&
          (!fullHeight || (result.top >= -1 && result.bottom <= result.height + 1)),
        label + ' fits the viewport',
      )
    }
    for (const variant of [
      {
        name: 'desktop',
        width: 1360,
        height: 900,
        mobile: false,
        session: 7,
        scope: 'workspace',
        primary: 'reviewer',
        subagent: 'explore',
      },
      {
        name: 'mobile',
        width: 390,
        height: 844,
        mobile: true,
        session: 42,
        scope: 'global',
        primary: 'reviewer',
        subagent: 'explore',
      },
    ]) {
      await cdp.send('Emulation.setDeviceMetricsOverride', {
        width: variant.width,
        height: variant.height,
        deviceScaleFactor: 1,
        mobile: variant.mobile,
      })
      await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: variant.mobile, maxTouchPoints: 1 })
      await cdp.send('Page.navigate', { url: base + '/chat?sessionId=' + variant.session })
      await composerReady(variant)
      await wait(async () => (await text(trigger)).toLowerCase().includes('main'), 'initial main role')
      await cdp.click(trigger)
      await wait(() => visible(item), 'role popup options')
      check(
        await cdp.evaluate("location.pathname==='/chat'"),
        variant.name + ' opens profile popup without leaving chat',
      )
      const popupText = await text(popup)
      check(
        popupText.includes('Agent profiles') &&
          popupText.includes('Reviewer') &&
          popupText.includes('Review correctness'),
        variant.name + ' lists profile names and descriptions',
      )
      check(
        !popupText.includes('Read the project and report focused evidence.'),
        variant.name + ' filters subagent-only roles from primary selection',
      )
      await layout(variant.name + ' profile popup', popup, true)
      await cdp.screenshot(path.join(evidence, variant.name + '-profile-popup.png'))
      const count = fixture.writes.length
      await cdp.clickText(item, 'Reviewer')
      await wait(() => fixture.writes.length > count, 'session role update')
      check(
        fixture.writes.at(-1).sessionId === variant.session && fixture.writes.at(-1).profile === 'reviewer',
        variant.name + ' selection persists only the current session profile',
      )
      await wait(
        async () => !(await visible(item)) && (await text(trigger)).toLowerCase().includes('reviewer'),
        'selected reviewer chip',
      )
      check(await cdp.evaluate("location.pathname==='/chat'"), variant.name + ' stays in chat after selection')
      await cdp.click(trigger)
      await cdp.clickText(item, 'Restore default role')
      await wait(() => fixture.writes.at(-1)?.profile === '', 'clear explicit role')
      await wait(async () => (await text(trigger)).toLowerCase().includes('main'), 'default main chip')
      check(
        fixture.writes.at(-1).sessionId === variant.session,
        variant.name + ' restores the default role through an empty profile update',
      )
      await cdp.click(trigger)
      await cdp.clickText(item, 'Default profile settings')
      await wait(() => visible(defaults), 'default profile settings region')
      check(
        await cdp.evaluate(
          `location.pathname==='/settings/runtime-session'&&new URLSearchParams(location.search).get('view')==='session-behavior'&&new URLSearchParams(location.search).get('scope')===${JSON.stringify(variant.scope)}`,
        ),
        variant.name + ' opens defaults for the correct settings scope',
      )
      check(
        await cdp.evaluate(
          "document.getElementById(location.hash.slice(1))?.dataset.testid === 'agent-profile-defaults'",
        ),
        variant.name + ' default settings anchor names the rendered region',
      )
      await layout(variant.name + ' default settings', defaults, false)
      for (const [kind, expected] of [
        ['primary', 'main'],
        ['subagent', 'worker'],
      ]) {
        await wait(
          async () => (await text('[data-testid="agent-profile-' + kind + '-effective"]')).trim() === expected,
          variant.name + ' initial ' + kind + ' default',
        )
        check(
          (await text('[data-testid="agent-profile-' + kind + '-effective"]')).trim() === expected,
          variant.name + ' shows the effective ' + kind + ' default for its own scope',
        )
      }
      for (const [kind, value, setting] of [
        ['primary', variant.primary, 'default_profile'],
        ['subagent', variant.subagent, 'default_subagent_profile'],
      ]) {
        const select = '[data-testid="agent-profile-' + kind + '-default"]'
        await wait(() => cdp.evaluate(`!(${live(select)})?.disabled`), 'loaded ' + kind + ' default')
        await chooseNative(select, value)
        if (variant.name === 'desktop' && kind === 'primary') {
          await cdp.evaluate(`void window.__profileProbe.router.replace({
            path: location.pathname,
            query: { ...Object.fromEntries(new URLSearchParams(location.search)), scope: 'global' },
            hash: location.hash
          })`)
          await wait(
            async () => (await text('[role="dialog"]')).includes('Discard unsaved changes?'),
            'default draft guard',
          )
          await cdp.clickText('[role="dialog"] button', 'Keep editing', true)
          check(
            await cdp.evaluate(
              `(${live(select)})?.value === ${JSON.stringify(value)} && location.search.includes('scope=workspace')`,
            ),
            'cancelling a scope switch keeps the unsaved default',
          )
        }
        const before = fixture.writes.length
        await cdp.click('[data-testid="agent-profile-save-' + kind + '-default"]')
        await wait(() => fixture.writes.length > before, 'saved ' + kind + ' default')
        const written = fixture.writes.at(-1)
        check(
          written.layer === variant.scope && written.path === setting && written.value === value,
          variant.name + ' saves ' + setting + ' to ' + variant.scope,
        )
        await wait(
          async () => (await text('[data-testid="agent-profile-' + kind + '-effective"]')).trim() === value,
          'saved effective ' + kind + ' default',
        )
      }
      await cdp.screenshot(path.join(evidence, variant.name + '-default-settings.png'))
      await navigate('/chat?sessionId=' + variant.session)
      await composerReady(variant)
      await cdp.click(trigger)
      await cdp.clickText(item, 'Default profile settings')
      await wait(() => visible(defaults), 'reopened defaults')
      for (const [kind, value] of [
        ['primary', variant.primary],
        ['subagent', variant.subagent],
      ]) {
        await wait(
          () =>
            cdp.evaluate(
              `(${live('[data-testid="agent-profile-' + kind + '-default"]')})?.value===${JSON.stringify(value)}`,
            ),
          'reopened ' + kind + ' default',
        )
        check(
          (await text('[data-testid="agent-profile-' + kind + '-effective"]')).trim() === value,
          variant.name + ' reopens with persisted ' + kind + ' default visible',
        )
      }
    }
    await navigate('/settings/runtime-session?view=agent-profiles&scope=workspace#default-agent-profiles')
    await wait(
      () =>
        cdp.evaluate(
          "location.search.includes('view=session-behavior') && location.search.includes('scope=workspace') && location.hash === '#settings-default-agent-profiles'",
        ),
      'legacy default bookmark redirect',
    )
    await wait(() => visible(defaults), 'legacy bookmark opens the new defaults card')
    check(await visible(defaults), 'legacy default settings link preserves the workspace scope')
    await navigate('/settings/runtime-session?view=session-behavior&scope=effective')
    await wait(
      async () => (await text('[data-testid="agent-profile-subagent-effective"]')).trim() === 'explore',
      'merged subagent default',
    )
    check(!(await visible(defaults + ' select')), 'effective scope displays the saved profile defaults read-only')
    check(errors.length === 0, 'No runtime browser exceptions')
    await fs.writeFile(
      path.join(evidence, 'result.json'),
      JSON.stringify({ checks, writes: fixture.writes, layouts, errors }, null, 2),
    )
    console.log(JSON.stringify({ ok: true, checks: checks.length, evidence }))
  } catch (error) {
    if (cdp) {
      await cdp.screenshot(path.join(evidence, 'failure.png')).catch(() => {})
      await fs.writeFile(
        path.join(evidence, 'failure.json'),
        JSON.stringify(
          {
            error: String(error),
            errors,
            requests: fixture.requests.slice(-80),
            writes: fixture.writes,
            page: await cdp.evaluate('({url:location.href,text:document.body.innerText})').catch(() => null),
            composers: await cdp.evaluate(composerInspection).catch(() => null),
          },
          null,
          2,
        ),
      )
    }
    throw error
  } finally {
    cdp?.close()
    chrome?.kill('SIGTERM')
    await fs.rm(temporary, { recursive: true, force: true })
    await fs.rm(chromeProfile, { recursive: true, force: true }).catch(() => {})
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
