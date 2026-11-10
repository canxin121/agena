// Real application and native Chrome DevTools protocol against an isolated in-memory API.
// No Agena service, user account, model calls, or project mutations. Run with Node.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const os = require('node:os')
const path = require('node:path')
const { spawn } = require('node:child_process')

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
  async clickText(selector, label) {
    const point = await wait(() => this.evaluate(`(() => {
      const n=[...document.querySelectorAll(${JSON.stringify(selector)})].find(el=>el.getClientRects().length && !el.disabled && el.textContent.includes(${JSON.stringify(label)}));
      if(!n) return null; n.scrollIntoView({block:'center'}); const r=n.getBoundingClientRect();
      const x=r.x+r.width/2,y=r.y+r.height/2; return n.contains(document.elementFromPoint(x,y)) ? {x,y,touch:matchMedia('(pointer:coarse)').matches} : null;
    })()`), `visible ${selector} containing ${label}`)
    if(point.touch) {
      await this.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x:point.x,y:point.y}]})
      await this.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]})
    } else {
      for(const type of ['mousePressed','mouseReleased']) await this.send('Input.dispatchMouseEvent',{type,x:point.x,y:point.y,button:'left',clickCount:1})
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

async function main() {
  assert(!process.versions.bun, 'Run with Node')
  const web = path.resolve(__dirname, '..')
  const evidence = process.env.BOT_EVIDENCE || '/tmp/agena-multi-bot'
  await fs.mkdir(evidence, { recursive: true })
  const temporary = await fs.mkdtemp(path.join(web, '.bot-probe-'))
  const profile = await fs.mkdtemp(path.join(os.tmpdir(), 'agena-bot-chrome-'))
  const requests = [],
    errors = [],
    checks = [],
    botActions = []
  let botParts = [],
    createdProject = false
  let server, chrome, cdp, base
  let botDelay = 0,
    botFailures = 0,
    botRequests = 0
  const botA = '11111111-1111-4111-8111-111111111111'
  const botB = '22222222-2222-4222-8222-222222222222'
  const botC = '33333333-3333-4333-8333-333333333333'
  const persona = () => ({ instructions: '' })
  const profileFor = (id, session_id, name) => ({
    id,
    session_id,
    name,
    persona: persona(),
    revision: 1,
    archived: false,
    created_at_ms: 1,
    updated_at_ms: 1,
  })
  const profiles = [profileFor(botA, 42, 'Atlas'), profileFor(botB, 43, 'Muse')]
  const profileRequests = [],
    sent = []
  const histories = new Map()
  let archiveConflict = false, sidebarWriteFailure = false, releaseLateCreate = null, holdAuthList = false, releaseAuthList = null
  const sidebarId = '44444444-4444-4444-8444-444444444444'
  const session = (id) => ({
    id,
    title:
      id === 42
        ? 'Server operator'
        : id === 43
          ? 'Muse conversation'
          : id === 44
            ? 'New Bot conversation'
            : id === 8
              ? 'Created project conversation'
              : 'Existing project conversation',
    workspace_id: id >= 42 ? null : 7,
    version: 1,
    state: { kind: 'ready', data: {} },
  })
  const execution = (id) => ({
    session: session(id),
    parts: id === 42 ? botParts : histories.get(id) || [],
    part_page: null,
    latest_event_seq: 1,
    queued_messages: [],
    queue_paused: false,
    background_activities: [],
    execution: { agent_status: 'completed' },
  })
  const provider = {
    plugin_id: 'example.source',
    fingerprint: 'fixture',
    adapter: {
      kind: 'event_source',
      title: 'Fixture events',
      description: 'In-memory external events',
      minimum_poll_seconds: 60,
      idempotent_send: false,
      config_schema: { properties: { repository: { type: 'string' } } },
    },
  }
  const policy = {
    identity: { tenant: 'fixture', account: 'bot', kind: 'group' },
    session_mode: 'per_user',
    thread_mode: 'exact',
    trigger: 'direct_or_mention',
    members: [{ actor: 'fixture-owner', role: 'owner' }],
    grants: {},
    service_grant: { tools: [], private_memory: false, source_workspace: false, network_hosts: [] },
    allow_unlisted_chat: false,
    allow_automated_reports: false,
    per_user_daily_limit: 50,
  }
  const spec = {
    title: 'Fixture channel',
    provider: 'example.channel',
    mode: 'channel',
    config: {},
    resource: 'fixture-room',
    event_types: ['message'],
    actors: [],
    instruction: '',
    poll_seconds: 60,
    expires_at_ms: Date.now() + 86400000,
    daily_limit: 5,
    total_limit: 50,
    max_execution_seconds: 300,
    use_source_permissions: false,
    channel_policy: policy,
  }
  const binding = (id, value) => ({
    id,
    spec: value,
    source_session_id: 7,
    workspace_id: 7,
    revision: 1,
    epoch: 1,
    state: 'disabled',
    request_key: id,
    admitted: 0,
    day_admitted: 0,
    day: 'fixture',
    cursor: null,
    created_at_ms: 1,
    updated_at_ms: 1,
    next_poll_ms: 0,
    poll_failures: 0,
    provider_fingerprint: 'fixture',
  })
  const bindings = [binding('channel-fixture', spec)]
  const accessRequests = []
  const check = (condition, description) => {
    assert(condition, description)
    checks.push(description)
    console.log(`PASS ${description}`)
  }
  const visible = (selector) =>
    cdp.evaluate(
      `!![...document.querySelectorAll(${JSON.stringify(selector)})].find(el=>el.getClientRects().length && getComputedStyle(el).visibility!=='hidden')`,
    )
  const active = () =>
    cdp.evaluate(
      `({path:location.pathname, session:new URLSearchParams(location.search).get('sessionId'), tabs:window.__botProbe.ui.workspaceWindows.map(t=>({id:t.id,tab:t.mainTab,query:t.routeQuery})), draft:window.__botProbe.chat.getComposerDraft('42')})`,
    )
  const navigate = (url) => cdp.evaluate(`window.__botProbe.router.push(${JSON.stringify(url)})`)
  const input = '[data-chat-input="true"]'
  try {
    await fs.writeFile(
      path.join(temporary, 'entry.ts'),
      `
import '/src/main.ts'
import { router } from '/src/router.ts'
import { useUiStore } from '/src/stores/ui.ts'
import { useChatStoreUnscoped } from '/src/stores/chat.ts'
import { setAppLocale } from '/src/i18n/index.ts'
import { useWorkspaceNavigation } from '/src/app/navigation/useWorkspaceNavigation.ts'
import { createApp, h } from 'vue'
import { getActivePinia } from 'pinia'
import Markdown from '/src/components/Markdown.vue'
import { i18n } from '/src/i18n/index.ts'
setAppLocale('zh-CN')
window.__botKeyboardTrace=[]
window.addEventListener('keydown',event=>{ if(['Enter','Escape'].includes(event.key))setTimeout(()=>window.__botKeyboardTrace.push({key:event.key,prevented:event.defaultPrevented,target:event.target?.tagName,dialogs:document.querySelectorAll('[role=dialog]').length,time:performance.now()}),0)},true)
window.__botProbe = {activeComposerSession: () => {
  let owner = [...document.querySelectorAll('[data-chat-input="true"]')].find(el=>el.getClientRects().length)?.__vueParentComponent;
  while(owner && !owner.type.__file?.endsWith('/pages/ChatPage.vue')) owner = owner.parent;
  return owner?.setupState.chat.selectedSessionId;
},router,ui:useUiStore(),chat:useChatStoreUnscoped(),navigation:useWorkspaceNavigation(), mountProjectLink: (absolute = false) => {
  const host = document.createElement('div'); host.id = 'bot-markdown-link';
  Object.assign(host.style, {position:'fixed',top:'80px',right:'40px',zIndex:'100'}); document.body.append(host);
  const href = (absolute ? location.origin : '') + '/chat?sessionId=7';
  const app = createApp({render:()=>h(Markdown,{content:'[Open project conversation]('+href+')',sessionId:'42'})});
  app.use(getActivePinia()); app.use(i18n); app.use(router); app.mount(host);
  return () => {app.unmount();host.remove()}
}}
`,
    )
    const { createServer } = await import('vite')
    server = await createServer({
      root: web,
      configFile: path.join(web, 'vite.config.ts'),
      logLevel: 'warn',
      server: { host: '127.0.0.1', port: 0, proxy: {}, hmr: false },
      plugins: [
        {
          name: 'bot-fixture',
          configureServer(dev) {
            dev.middlewares.use(async (req, res, next) => {
              const url = new URL(req.url, 'http://fixture'),
                p = url.pathname
              const json = (value, status = 200) => {
                res.statusCode = status
                res.setHeader('Content-Type', 'application/json')
                res.end(JSON.stringify(value))
              }
              if (/^\/(api|auth|health)(\/|$)/.test(p)) {
                requests.push({ method: req.method, path: p })
                if (/^\/api\/v1\/(tasks(?:\/|$)|chatbot\/messages$)/.test(p))
                  return json({ error: 'Retired endpoint' }, 404)
                if (/^\/api\/v1\/sessions\/\d+\/messages$/.test(p) && req.method === 'POST') {
                  let raw = ''
                  for await (const chunk of req) raw += chunk
                  const targetId = Number(p.split('/')[4])
                  sent.push({ path: p, body: JSON.parse(raw) })
                  if (targetId !== 42) {
                    histories.set(targetId, [
                      {
                        part_id: targetId * 100 + sent.length,
                        run_id: targetId * 100 + sent.length,
                        revision: 1,
                        message_boundary: true,
                        role: 'assistant',
                        kind: 'text',
                        state: 'completed',
                        sections: [],
                        content: {
                          text: `History only for ${targetId}: ${raw.includes('Muse message') ? 'Muse message' : 'Bot message'}`,
                        },
                        created_at_ms: 1,
                        updated_at_ms: 1,
                      },
                    ])
                    return json(execution(targetId))
                  }
                  const create = raw.includes('Create a regular project session')
                  const list = raw.includes('List project sessions')
                  if (create || list) {
                    if (create) createdProject = true
                    botActions.push({ tool: 'bot.sessions', operation: create ? 'create' : 'list' })
                    const text = create
                      ? '[Created project conversation](/chat?sessionId=8)'
                      : '[Existing project conversation](/chat?sessionId=7)\n\n[Created project conversation](/chat?sessionId=8)'
                    botParts = [
                      {
                        part_id: 100,
                        run_id: 100,
                        revision: botActions.length,
                        message_boundary: true,
                        role: 'assistant',
                        kind: 'text',
                        state: 'completed',
                        sections: [],
                        content: { text },
                        created_at_ms: 1,
                        updated_at_ms: botActions.length,
                      },
                    ]
                  }
                  return json(execution(42))
                }
                if (p.endsWith('/stream') || p.endsWith('/watch')) {
                  res.writeHead(200, { 'Content-Type': 'text/event-stream' })
                  res.write(': fixture\n\n')
                  return
                }
                const integration = /^\/api\/v1\/sessions\/7\/integrations(?:\/([^/]+))?(?:\/(control|access))?$/.exec(
                  p,
                )
                if (integration) {
                  const [, id, action] = integration
                  let body = {}
                  if (req.method !== 'GET') {
                    let raw = ''
                    for await (const chunk of req) raw += chunk
                    body = JSON.parse(raw || '{}')
                  }
                  if (!id && req.method === 'GET')
                    return json({ source_session_id: 7, workspace_id: 7, providers: [provider], bindings })
                  if (!id && req.method === 'POST') {
                    const created = binding('session-fixture', body.spec)
                    bindings.push(created)
                    return json(created)
                  }
                  const current = bindings.find((item) => item.id === id)
                  if (!current) return json({ error: 'Unknown binding' }, 404)
                  if (action) {
                    if (body.expected_revision !== current.revision) return json({ error: 'Revision conflict' }, 409)
                    if (action === 'control') current.state = body.action === 'enable' ? 'enabled' : 'paused'
                    if (action === 'access') {
                      current.spec.channel_policy = body.policy
                      accessRequests.push(body)
                    }
                    current.revision++
                    return json(current)
                  }
                  return json({
                    binding: current,
                    channel_sessions: [],
                    access_audit: [],
                    deliveries: [],
                    events: [
                      {
                        binding_id: id,
                        epoch: 1,
                        state: 'completed',
                        received_at_ms: 1,
                        execution_id: 'fixture-execution',
                        session_id: 8,
                        execution_generation: 4,
                        event: {
                          id: 'fixture-event',
                          actor: 'fixture-owner',
                          text: 'Completed ordinary session execution',
                        },
                      },
                    ],
                  })
                }
                if (/^\/api\/v1\/sessions\/\d+\/assistant$/.test(p))
                  return json({
                    profile: {
                      workspace_id: null,
                      revision: 1,
                      preferences: {
                        name: 'Fixture assistant',
                        address_user: 'Operator',
                        tone: 'balanced',
                        language: 'auto',
                        instructions: '',
                        memory_enabled: true,
                        proactive_enabled: false,
                        timezone: 'UTC',
                        quiet_enabled: false,
                        quiet_start_minute: 0,
                        quiet_end_minute: 0,
                        daily_limit: 5,
                        cooldown_seconds: 60,
                      },
                    },
                    memories: [],
                    memories_truncated: false,
                    reminders: [],
                  })
                if (p === '/api/v1/bots' && req.method === 'GET') {
                  if (holdAuthList) await new Promise(resolve => { releaseAuthList = resolve })
                  return json({
                    items: profiles.filter((bot) => !bot.archived),
                    default_bot_id: profiles.find((bot) => !bot.archived)?.id || null,
                  })
                }
                const botRoute = /^\/api\/v1\/bots\/([^/]+)(?:\/(session|archive))?$/.exec(p)
                if (p === '/api/v1/bots' || botRoute) {
                  const [, id, action] = botRoute || []
                  const current = profiles.find((bot) => bot.id === id)
                  if (req.method === 'GET') return current ? json(current) : json({ error: 'Unknown Bot' }, 404)
                  if (action !== 'session') {
                    let raw = ''
                    for await (const chunk of req) raw += chunk
                    const body = JSON.parse(raw || '{}')
                    profileRequests.push({ path: p, method: req.method, body })
                    if (sidebarWriteFailure) { sidebarWriteFailure = false; return json({ error: 'Fixture network failure' }, 503) }
                    if (!body.request_key) return json({ error: 'request_key required' }, 400)
                    if (!id) {
                      if (body.name === 'Late Sidebar Bot') await new Promise(resolve => { releaseLateCreate = resolve })
                      const created = { ...profileFor(body.name === 'Sidebar Orbit' ? sidebarId : body.name === 'Late Sidebar Bot' ? '66666666-6666-4666-8666-666666666666' : botC, body.name === 'Sidebar Orbit' ? 45 : body.name === 'Late Sidebar Bot' ? 46 : 44, body.name), persona: body.persona || persona() }
                      profiles.push(created)
                      return json(created)
                    }
                    if (!current) return json({ error: 'Unknown Bot' }, 404)
                    if (body.expected_revision !== current.revision) return json({ error: 'Revision conflict' }, 409)
                    if (action === 'archive') {
                      if (archiveConflict) {
                        archiveConflict = false
                        return json({ error: 'Bot must be idle: queued work remains' }, 409)
                      }
                      current.archived = true
                    } else {
                      current.name = body.name
                      current.persona = body.persona
                    }
                    current.revision++
                    return json(current)
                  }
                }
                if (p === '/api/v1/bot/session' || botRoute?.[2] === 'session') {
                  botRequests++
                  const current = botRoute
                    ? profiles.find((bot) => bot.id === botRoute[1])
                    : profiles.find((bot) => !bot.archived)
                  const fail = botFailures-- > 0,
                    delay = botDelay
                  if (delay) await sleep(delay)
                  if (!current || current.archived)
                    return json({ error: 'Unknown or archived Bot. Choose or create a Bot.' }, 404)
                  return json(
                    fail
                      ? { error: 'Fixture temporarily unavailable' }
                      : { session_id: current.session_id, bot_id: current.id, name: current.name },
                    fail ? 503 : 200,
                  )
                }
                if (p === '/auth/session') return json({ authenticated: true, disabled: true })
                if (p === '/api/v1/health') return json({ status: 'ok', database_connected: true })
                if (p === '/api/v1/runtime')
                  return json({
                    default_selection: { provider: 'fixture', adapter: 'local', model: 'fixture-model' },
                    workspace: null,
                  })
                if (p === '/api/v1/providers') return json([{ provider_id: 'fixture' }])
                if (p.endsWith('/configured-models'))
                  return json([
                    {
                      adapter_id: 'local',
                      enabled: true,
                      models: [
                        { id: 'fixture-model', display_name: 'Fixture Model' },
                        { id: 'alternate-model', display_name: 'Alternate Model' },
                      ],
                    },
                  ])
                if (p === '/api/v1/plugins/surface')
                  return json({
                    catalog: {
                      commands: [
                        {
                          id: 'assistant',
                          slash: '/assistant',
                          aliases: [],
                          target: { kind: 'client', action: 'assistant' },
                          accepts_empty_input: true,
                          docs: { summary: 'Assistant settings' },
                        },
                        {
                          id: 'bot',
                          slash: '/bot',
                          aliases: [],
                          target: { kind: 'client', action: 'bot' },
                          accepts_empty_input: true,
                          docs: { summary: 'Open Bot' },
                        },
                      ],
                    },
                  })
                if (p === '/api/v1/settings') return json({ value: {} })
                if (p === '/api/v1/sessions')
                  return json({
                    items: [session(7), ...(createdProject ? [session(8)] : [])],
                    total: createdProject ? 2 : 1,
                    page: { has_more: false },
                  })
                if (/^\/api\/v1\/sessions\/\d+$/.test(p)) return json(session(Number(p.split('/').at(-1))))
                if (/\/sessions\/\d+\/(parts|runs)$/.test(p))
                  return json({
                    session_id: Number(p.split('/')[4]),
                    parts: Number(p.split('/')[4]) === 42 ? botParts : histories.get(Number(p.split('/')[4])) || [],
                    runs: [],
                    user_message_count: 0,
                    page: { has_more: false },
                    version: 1,
                  })
                if (/\/sessions\/\d+\/(state|messages)$/.test(p)) return json(execution(Number(p.split('/')[4])))
                if (p === '/api/v1/workspaces/7') return json({ id: 7, path: '/fixture/project' })
                if (p === '/api/v1/workbench/fs/home') return json({ home: '/fixture' })
                if (p === '/api/v1/activities') return json({ items: [], activities: [] })
                if (p === '/api/v1/workspaces') return json({ items: [], page: { has_more: false } })
                if (req.method !== 'GET') return json({ error: 'Unexpected fixture mutation' }, 400)
                return json({ items: [], page: { has_more: false } })
              }
              if (!['/', '/bot', '/chat', '/assistant', '/integrations', '/tasks'].includes(p)) return next()
              const html =
                '<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="app"></div><script type="module" src="/@fs/' +
                path.join(temporary, 'entry.ts') +
                '"></script></body></html>'
              res.setHeader('Content-Type', 'text/html')
              res.end(await dev.transformIndexHtml(p, html))
            })
          },
        },
      ],
    })
    await server.listen()
    base = `http://127.0.0.1:${server.httpServer.address().port}`
    chrome = spawn(
      process.env.CHROME_BIN || '/usr/bin/google-chrome',
      [
        '--headless=new',
        '--no-sandbox',
        '--disable-dev-shm-usage',
        '--disable-background-networking',
        '--disable-extensions',
        '--remote-debugging-port=0',
        `--user-data-dir=${profile}`,
        'about:blank',
      ],
      { stdio: ['ignore', 'ignore', 'ignore'] },
    )
    const port = await wait(
      async () => Number((await fs.readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]),
      'Chrome startup',
    )
    const target = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: 'PUT' })).json()
    const socket = new WebSocket(target.webSocketDebuggerUrl)
    await new Promise((resolve, reject) => {
      socket.addEventListener('open', resolve, { once: true })
      socket.addEventListener('error', reject, { once: true })
    })
    cdp = new Cdp(socket)
    cdp.events.set('Runtime.exceptionThrown', (v) =>
      errors.push(v.exceptionDetails.exception?.description || JSON.stringify(v)),
    )
    cdp.events.set('Fetch.requestPaused', (v) => {
      const url = new URL(v.request.url),
        local = url.origin === base || ['data:', 'blob:'].includes(url.protocol)
      void cdp
        .send(local ? 'Fetch.continueRequest' : 'Fetch.failRequest', {
          requestId: v.requestId,
          ...(!local ? { errorReason: 'BlockedByClient' } : {}),
        })
        .catch(() => {})
    })
    await cdp.send('Runtime.enable')
    await cdp.send('Page.enable')
    await cdp.send('Page.bringToFront')
    await cdp.send('Fetch.enable', { patterns: [{ urlPattern: '*' }] })
    await cdp.send('Emulation.setDeviceMetricsOverride', {
      width: 1360,
      height: 900,
      deviceScaleFactor: 1,
      mobile: false,
    })
    // Exercise registry management before entering a Bot conversation. All clicks
    // use hit-tested CDP input; hidden kept-alive panes are never selected.
    await cdp.send('Page.navigate', { url: base + '/chat?sessionId=7' })
    await wait(() => visible('[data-testid="sidebar-bots"]'), 'project Bot section', 90000)
    const row = (id) => `[data-sidebar-bot-id="${id}"]`
    await wait(() => visible(`${row(botA)} [data-oc-list-item-primary]`), 'Atlas sidebar row')
    await wait(() => visible(`${row(botB)} [data-oc-list-item-primary]`), 'Muse sidebar row')
    const registryOnly = requests.length
    const route = await cdp.evaluate('location.pathname+location.search')
    check(await cdp.evaluate(`(() => {const s=[...document.querySelectorAll('[data-testid="sidebar-bots"]')].find(n=>n.getClientRects().length);return s.innerText.includes('Atlas') && s.innerText.includes('Muse') && s.innerText.includes('2')})()`), 'project sidebar displays named rows and registry count')
    for (const [query, expected, absent] of [['atlas',botA,botB], [' MUSE ',botB,botA], ['not a bot',null,botA], ['',botA,null]]) {
      await cdp.fill('[data-testid="sidebar-bot-search"]', query)
      await wait(async()=> expected ? (await visible(row(expected))) && (!absent || !(await visible(row(absent)))) : !(await visible(row(absent))), `sidebar search ${query}`)
    }
    check(await visible(row(botB)), 'sidebar search filters case and spaces, handles no matches and clears')
    await cdp.click('[data-testid="sidebar-bot-create"]')
    await wait(() => visible('[data-testid="bot-persona-form"]'), 'sidebar create dialog')
    await cdp.fill('[data-testid="bot-edit-name"]', '   ')
    const disabled=await cdp.evaluate(`([...document.querySelectorAll('[data-testid="bot-save"]')].find(n=>n.getClientRects().length))?.disabled`)
    if (!disabled) await cdp.click('[data-testid="bot-save"]')
    check(profileRequests.length===0, 'empty or whitespace Bot name cannot be submitted')
    await cdp.key('Escape','Escape',27)
    await wait(async()=>!(await visible('[data-testid="bot-persona-form"]')), 'cancel create')
    check(profileRequests.length===0 && await cdp.evaluate('location.pathname+location.search')===route, 'cancel create makes no profile mutation or navigation')
    await cdp.click(`${row(botB)} [data-testid="sidebar-bot-actions"]`)
    await cdp.key('Escape','Escape',27)
    await wait(()=>cdp.evaluate(`document.activeElement?.matches(${JSON.stringify(`${row(botB)} [data-testid="sidebar-bot-actions"]`)})`), 'row menu Escape focus return')
    check(true, 'keyboard Escape closes row menu and restores focus')
    await cdp.click(`${row(botB)} [data-testid="sidebar-bot-actions"]`)
    await cdp.clickText('[data-option-menu-item]', 'Edit Bot')
    await wait(()=>visible('[data-testid="bot-edit-name"]'), 'sidebar edit ready')
    profiles[1].revision++
    profiles[1].persona.instructions='Remote update'
    await cdp.fill('[data-testid="bot-edit-instructions"]', 'Local unsaved draft')
    await cdp.click('[data-testid="bot-save"]')
    await wait(()=>visible('[role="dialog"] [role="alert"]'), 'stale sidebar edit conflict')
    check(await cdp.evaluate(`([...document.querySelectorAll('[data-testid="bot-edit-instructions"]')].find(n=>n.getClientRects().length))?.value==='Local unsaved draft'`), 'revision conflict preserves local persona draft')
    const reloadLabel=await cdp.evaluate(`import('/src/i18n/index.ts').then(({i18n})=>i18n.global.t('bot.profiles.reload'))`)
    await cdp.clickText('[role="dialog"] button',reloadLabel)
    await wait(()=>cdp.evaluate(`([...document.querySelectorAll('[data-testid="bot-edit-instructions"]')].find(n=>n.getClientRects().length))?.value==='Remote update'`), 'explicitly reload remote persona')
    check(true, 'explicit reload reconciles stale profile')
    await cdp.fill('[data-testid="bot-edit-name"]', 'Muse Sidebar')
    await cdp.fill('[data-testid="bot-edit-instructions"]', 'Sidebar persona')
    await cdp.click('[data-testid="bot-save"]')
    await wait(()=>profiles[1].name==='Muse Sidebar', 'unselected row edit saved')
    await wait(()=>visible(`${row(botB)} [data-oc-list-item-primary]`), 'edited row')
    await wait(async()=>!(await visible('[data-testid="bot-persona-form"]')), 'saved editor closes')
    check(await cdp.evaluate(`location.pathname+location.search===${JSON.stringify(route)} && [...document.querySelectorAll('[role="dialog"]')].filter(n=>n.getClientRects().length).length===0`), 'row edit updates unselected profile without navigation or duplicate dialogs')
    check(profiles[0].name==='Atlas' && profiles[1].persona.instructions==='Sidebar persona', 'sidebar edit preserves other profile and saves persona')
    check(!requests.slice(registryOnly).some(r => /\/sessions\/\d+\/(messages|model)|\/bots\/[^/]+\/session|\/bot\/session/.test(r.path)), 'list, search, cancel and edit do not create sessions or send model messages')
    const crowded = Array.from({length:24}, (_,i)=>profileFor(`55555555-5555-4555-8555-${String(i).padStart(12,'0')}`, 100+i, i===23 ? 'مستكشف طويل للغاية Sidebar RTL Name' : `Crowded Bot ${i}`))
    profiles.push(...crowded)
    await cdp.click('[data-testid="sidebar-bot-refresh"]')
    await wait(()=>cdp.evaluate(`(() => {const s=[...document.querySelectorAll('[data-testid="sidebar-bots"]')].find(n=>n.getClientRects().length);const l=s?.querySelector('ul');return l && l.scrollHeight>l.clientHeight && l.clientHeight<=220})()`), 'bounded many Bot list')
    await cdp.fill('[data-testid="sidebar-bot-search"]', 'مستكشف طويل')
    await wait(()=>visible(row(crowded[23].id)), 'long RTL search')
    check(await cdp.evaluate('document.documentElement.scrollWidth<=innerWidth+1'), 'many Bots have bounded scroll and long RTL name is searchable without overflow')
    await cdp.fill('[data-testid="sidebar-bot-search"]', '')
    profiles.splice(profiles.indexOf(crowded[0]),crowded.length)
    await cdp.click('[data-testid="sidebar-bot-refresh"]')
    await wait(async()=>!(await visible(row(crowded[0].id))), 'crowded fixture removed')
    await cdp.click('[data-testid="sidebar-bot-create"]')
    await cdp.fill('[data-testid="bot-edit-name"]', 'Sidebar Orbit')
    await cdp.fill('[data-testid="bot-edit-instructions"]', 'Independent sidebar persona')
    sidebarWriteFailure = true
    await cdp.click('[data-testid="bot-save"]')
    await wait(()=>visible('[role="dialog"] [role="alert"]'), 'sidebar create failure')
    await cdp.click('[data-testid="bot-save"]')
    await wait(()=>profiles.some(p=>p.id===sidebarId), 'sidebar create retry')
    await wait(()=>cdp.evaluate(`new URLSearchParams(location.search).get('botId')==='${sidebarId}'`), 'created Bot selected')
    const attempts=profileRequests.filter(r=>r.body.name==='Sidebar Orbit')
    check(attempts.length===2 && attempts[0].body.request_key===attempts[1].body.request_key && profiles.filter(p=>p.id===sidebarId).length===1, 'failed create retries with stable key and creates exactly one Bot')
    await cdp.click(`${row(sidebarId)} [data-testid="sidebar-bot-actions"]`)
    await cdp.clickText('[data-option-menu-item]', 'Archive Bot')
    await cdp.key('Escape','Escape',27)
    await wait(async()=>!(await visible('[data-testid="bot-archive-confirm"]')), 'cancel archive')
    check(!profiles.find(p=>p.id===sidebarId).archived, 'archive cancel retains row')
    await cdp.click(`${row(sidebarId)} [data-testid="sidebar-bot-actions"]`)
    await cdp.clickText('[data-option-menu-item]', 'Archive Bot')
    archiveConflict=true
    await cdp.click('[data-testid="bot-archive-confirm"]')
    await wait(()=>visible('[role="dialog"] [role="alert"]'), 'sidebar busy conflict')
    check(!profiles.find(p=>p.id===sidebarId).archived && await visible(row(sidebarId)), 'busy archive leaves Bot in sidebar')
    await cdp.click('[data-testid="bot-archive-confirm"]')
    await wait(()=>profiles.find(p=>p.id===sidebarId).archived, 'sidebar archive')
    await wait(async()=>!(await visible(row(sidebarId))), 'archived row removed')
    const sidebarArchives=profileRequests.filter(r=>r.path===`/api/v1/bots/${sidebarId}/archive`)
    check(sidebarArchives.length===2 && sidebarArchives[0].body.request_key===sidebarArchives[1].body.request_key && sidebarArchives[0].body.expected_revision===sidebarArchives[1].body.expected_revision, 'sidebar archive retries preserve revision and request key')
    await navigate('/chat?sessionId=7')
    await wait(()=>cdp.evaluate(`location.pathname==='/chat'`), 'project before delayed creation')
    await cdp.click('[data-testid="sidebar-bot-create"]')
    await cdp.fill('[data-testid="bot-edit-name"]','Late Sidebar Bot')
    await cdp.click('[data-testid="bot-save"]')
    await wait(()=>releaseLateCreate, 'delayed create request received')
    await navigate('/integrations?source=7')
    await wait(()=>cdp.evaluate(`location.pathname==='/integrations'`), 'navigate away during create')
    releaseLateCreate()
    await wait(()=>profiles.some(p=>p.name==='Late Sidebar Bot'), 'late create server response')
    await wait(()=>visible('[data-sidebar-bot-id="66666666-6666-4666-8666-666666666666"]'), 'late-created sidebar row')
    check(await cdp.evaluate(`location.pathname==='/integrations'`), 'late create response does not steal navigation')
    profiles.splice(profiles.findIndex(p=>p.name==='Late Sidebar Bot'),1)
    holdAuthList=true
    const cleared=await cdp.evaluate(`Promise.all([import('/src/lib/serverAuthToken.ts'),import('/src/stores/bots.ts')]).then(([auth,store])=>{auth.writeServerAuthTokenForBaseUrl(location.origin,'fixture-new-account'); return store.useBotsStore().profiles.length===0})`)
    await wait(()=>releaseAuthList, 'new auth list request')
    check(cleared, 'auth change clears old Bot registry before new list returns')
    holdAuthList=false
    releaseAuthList()
    await wait(()=>visible(row(botA)), 'new auth registry loaded')
    // Restore independent baseline for the original 85 acceptance checks.
    profiles.splice(profiles.findIndex(p=>p.id===sidebarId),1)
    Object.assign(profiles[1],profileFor(botB,43,'Muse'))
    await cdp.evaluate('localStorage.clear(); sessionStorage.clear()')
    await cdp.send('Page.navigate', { url: base + '/bot?sessionId=7' })
    await wait(() => visible('[data-testid="bot-welcome"]'), 'canonical Bot chat', 90000)
    check(
      (await active()).session === '42',
      'direct Bot reload ignores an unrelated session query and binds canonical session 42',
    )
    check(
      (await visible('[data-main-nav-tab="bot"]')) && (await visible('[data-testid="sidebar-bot"]')),
      'desktop rail and focused sidebar both prominently expose Bot',
    )
    check(await visible('[data-testid="bot-sidebar"]'), 'desktop Bot uses the standalone sidebar')
    check(await cdp.evaluate(`!document.querySelector('[data-keyboard-item^="directory:"], [data-keyboard-item^="session:"]')`), 'focused Bot mounts no project or session rows')
    check(await cdp.evaluate(`![...document.querySelectorAll('[data-workspace-pane-window]')].some(n=>n.getClientRects().length && n.innerText.includes('/fixture/project'))`), 'Bot conversation does not display the prior project directory')
    check(await visible(input), 'Bot uses the full existing chat composer')
    await cdp.evaluate(`document.querySelector('[data-testid="bot-selector"]').focus()`)
    await cdp.key('Enter', 'Enter', 13)
    await wait(() => visible('[data-testid="bot-manager"]'), 'keyboard named selector')
    await wait(() => cdp.evaluate(`!!document.activeElement?.closest('[role="dialog"]')`), 'keyboard dialog focus')
    await sleep(180) // Native dialog entrance animation and dismissable-layer registration.
    await cdp.key('Escape', 'Escape', 27)
    await wait(
      () => cdp.evaluate(`document.activeElement?.matches('[data-testid="bot-selector"]')`),
      'selector keyboard return focus',
    )
    check(true, 'named selector opens with Enter and returns focus with Escape')

    check(
      await cdp.evaluate(
        `!document.querySelector('[data-main-nav-tab="tasks"], [data-testid="chat-tasks"], a[href^="/tasks"]')`,
      ),
      'Task is absent from desktop navigation and chat actions',
    )
    await cdp.click('[data-testid="bot-settings"]')
    await wait(() => visible('[data-testid="bot-edit-name"]'), 'Bot personality editor')
    check(
      await cdp.evaluate(
        `window.__botProbe.ui.workspaceWindows.some(t=>t.mainTab==='bot' && t.routePath==='/bot' && t.routeQuery.sessionId==='42')`,
      ),
      'Bot settings open this Bot personality editor without replacing its session tab',
    )
    await cdp.key('Escape', 'Escape', 27)
    await wait(async () => !(await visible('[data-testid="bot-persona-form"]')), 'close Bot settings')
    await navigate('/tasks?source=7')
    await wait(() => cdp.evaluate(`location.pathname==='/'`), 'retired route falls through to workspace')
    check(
      await cdp.evaluate(
        `!window.__botProbe.router.getRoutes().some(r=>r.path==='/tasks') && !window.__botProbe.ui.workspaceWindows.some(t=>t.mainTab==='tasks')`,
      ),
      'Task has no registered route or workspace tab',
    )
    await navigate('/bot')
    await wait(() => visible('[data-testid="bot-welcome"]'), 'Bot after retired route')
    const beforeSuggestions = requests.filter((r) => r.path.endsWith('/messages')).length
    await cdp.click('[data-bot-suggestion="explore"]')
    await wait(
      () => cdp.evaluate(`window.__botProbe.chat.getComposerDraft('42').includes('列出我的项目')`),
      'suggestion draft',
    )
    check(
      requests.filter((r) => r.path.endsWith('/messages')).length === beforeSuggestions,
      'starting suggestion fills the draft without executing',
    )
    check(
      await cdp.evaluate(`document.activeElement?.matches('[data-chat-input="true"]')`),
      'suggestion moves keyboard focus into the existing composer',
    )
    await cdp.fill(input, 'Bot draft survives project navigation')
    // Select a non-default model through the real picker before navigating.
    await cdp.clickText('button', 'Fixture Model')
    await wait(() => visible('[data-option-menu-item]'), 'model picker')
    await cdp.clickText('[data-option-menu-item]', 'Alternate Model')
    await wait(() => cdp.evaluate(`document.body.innerText.includes('Alternate Model')`), 'alternate model')
    await cdp.evaluate(`(() => {
      const editor=[...document.querySelectorAll('[data-chat-input="true"]')].find(el=>el.getClientRects().length);
      const data=new DataTransfer();data.items.add(new File(['isolated fixture attachment'],'bot-note.txt',{type:'text/plain'}));
      editor.dispatchEvent(new ClipboardEvent('paste',{clipboardData:data,bubbles:true,cancelable:true}));
    })()`)
    await wait(() => visible('[data-chat-input="true"] [data-attachment-id]'), 'Bot attachment draft')
    await cdp.screenshot(path.join(evidence, 'web-bot-desktop.png'))
    const sendCurrent = async (text, targetId) => {
      const before = sent.length
      await cdp.fill(input, text)
      for (const type of ['keyDown', 'keyUp'])
        await cdp.send('Input.dispatchKeyEvent', {
          type,
          key: 'Enter',
          code: 'Enter',
          windowsVirtualKeyCode: 13,
          modifiers: 2,
        })
      await wait(() => sent.length > before, `outgoing message to ${targetId}`)
      check(
        sent.at(-1).path === `/api/v1/sessions/${targetId}/messages`,
        `native composer sends to exact session ${targetId}`,
      )
    }
    const chooseBot = async (id, sid) => {
      await cdp.click('[data-testid="bot-selector"]')
      await wait(() => visible(`[data-bot-select="${id}"]`), 'named list option')
      await cdp.click(`[data-bot-select="${id}"]`)
      await wait(
        () =>
          cdp.evaluate(
            `new URLSearchParams(location.search).get('botId')===${JSON.stringify(id)} && window.__botProbe.activeComposerSession()===${JSON.stringify(String(sid))}`,
          ),
        `binding ${id}`,
      )
      await wait(() => visible(input), 'native named composer')
    }
    await chooseBot(botB, 43)
    check(!(await visible('[data-chat-input="true"] [data-attachment-id]')), 'Atlas attachment does not leak to Muse')
    check(
      await cdp.evaluate(
        `window.__botProbe.chat.getComposerDraft('43')==='' && document.body.innerText.includes('Fixture Model')`,
      ),
      'Muse starts with its own empty draft and model',
    )
    await sendCurrent('Muse message one', 43)
    check(
      JSON.stringify(sent.at(-1).body).includes('fixture-model'),
      'Muse outgoing request uses its independently selected model',
    )
    await wait(() => cdp.evaluate(`document.body.innerText.includes('History only for 43')`), 'Muse history')
    await cdp.fill(input, 'Muse private draft')
    await chooseBot(botA, 42)
    check((await active()).draft === 'Bot draft survives project navigation', 'switching back restores Atlas draft')
    check(await visible('[data-chat-input="true"] [data-attachment-id]'), 'switching back restores Atlas attachment')
    check(
      await cdp.evaluate(
        `document.body.innerText.includes('Alternate Model') && !document.body.innerText.includes('History only for 43')`,
      ),
      'Atlas retains its model and excludes Muse history',
    )
    const beforeStaleBot = sent.length
    await cdp.evaluate(`history.replaceState(history.state, '', '/bot?botId=${botB}&sessionId=42')`)
    for (const type of ['keyDown', 'keyUp'])
      await cdp.send('Input.dispatchKeyEvent', {
        type,
        key: 'Enter',
        code: 'Enter',
        windowsVirtualKeyCode: 13,
        modifiers: 2,
      })
    await sleep(150)
    check(
      sent.length === beforeStaleBot && (await active()).draft === 'Bot draft survives project navigation',
      'same sessionId with a different botId blocks stale cross-Bot sends',
    )
    await cdp.evaluate(`history.replaceState(history.state, '', window.__botProbe.router.currentRoute.value.fullPath)`)
    await cdp.click('[data-testid="bot-selector"]')
    await cdp.click('[data-testid="bot-create"]')
    await cdp.fill('[data-testid="bot-edit-name"]', 'Orbit')
    await cdp.click('[data-testid="bot-save"]')
    await wait(() => cdp.evaluate(`window.__botProbe.activeComposerSession()==='44'`), 'post-create switch')
    check(
      JSON.stringify(profiles[2].persona) === JSON.stringify({ instructions: '' }),
      'new Bot starts with empty instructions only',
    )
    await cdp.click('[data-testid="bot-settings"]')
    await wait(() => visible('[data-testid="bot-edit-name"]'), 'edit new Bot')
    await cdp.fill('[data-testid="bot-edit-name"]', 'Orbit Studio')
    for (const selector of ['bot-edit-address', 'bot-edit-language', 'bot-edit-tone']) {
      check(!(await visible(`[data-testid="${selector}"]`)), `${selector} is absent`)
    }
    await cdp.fill('[data-testid="bot-edit-instructions"]', 'Use concise writing examples for this Bot only.')
    await cdp.click('[data-testid="bot-save"]')
    await wait(
      () => cdp.evaluate(`document.querySelector('[data-workspace-pane-window="'+window.__botProbe.ui.activeWorkspaceWindowId+'"] [data-testid="bot-name"]')?.innerText==='Orbit Studio'`),
      'renamed header',
    )
    check(
      JSON.stringify(profiles[2].persona) === JSON.stringify({ instructions: 'Use concise writing examples for this Bot only.' }) &&
        profiles[0].persona.instructions === '',
      'persona edits are explicit and scoped to Orbit',
    )
    check(
      await cdp.evaluate(
        `window.__botProbe.ui.workspaceWindows.some(tab=>tab.routeQuery.botId==='${botC}' && tab.title==='Orbit Studio')`,
      ),
      'rename updates named workspace tab',
    )
    await sendCurrent('Orbit message', 44)
    await cdp.click('[data-testid="bot-selector"]')
    await cdp.click(`[data-bot-archive="${botC}"]`)
    check(!profiles[2].archived, 'opening explicit archive confirmation has no mutation')
    archiveConflict = true
    await cdp.click('[data-testid="bot-archive-confirm"]')
    await wait(
      () => cdp.evaluate(`document.querySelector('[role="alert"]')?.innerText.includes('idle')`),
      'idle conflict shown',
    )
    check(!profiles[2].archived, 'server idle conflict preserves selected Bot')
    await cdp.click('[data-testid="bot-archive-confirm"]')
    await wait(() => profiles[2].archived, 'archive succeeds')
    await wait(() => visible('[data-testid="bot-loading"] [role="alert"]'), 'archived selected UUID error')
    check(
      (await cdp.evaluate(`new URLSearchParams(location.search).get('botId')==='${botC}'`)) && !(await visible(input)),
      'archive preserves selected UUID and never silently exposes another Bot composer',
    )
    const archiveCalls = profileRequests.filter((r) => r.path === `/api/v1/bots/${botC}/archive`)
    check(
      archiveCalls.length === 2 && archiveCalls[0].body.request_key === archiveCalls[1].body.request_key,
      'archive retries keep stable request_key and expected_revision',
    )
    await cdp.screenshot(path.join(evidence, 'web-bot-archived.png'))
    await cdp.key('Escape', 'Escape', 27)
    await chooseBot(botA, 42)
    check(
      !(await cdp.evaluate(`document.body.innerText.includes('History only for 44')`)),
      'archived Bot history never appears in Atlas',
    )
    await cdp.screenshot(path.join(evidence, 'web-bot-named-desktop.png'))
    const firstTabs = (await active()).tabs.filter((t) => t.tab === 'bot').length
    await cdp.click('[data-main-nav-tab="bot"]')
    await cdp.click('[data-main-nav-tab="bot"]')
    check(
      (await active()).tabs.filter((t) => t.tab === 'bot').length === firstTabs,
      'repeated desktop Bot entry is idempotent',
    )
    await navigate('/chat?sessionId=7')
    await wait(
      () =>
        cdp.evaluate(
          `location.pathname==='/chat' && !![...document.querySelectorAll('[data-chat-input="true"]')].find(el=>el.getClientRects().length)`,
        ),
      'project session',
    )
    check(
      !(await visible('[data-chat-input="true"] [data-attachment-id]')),
      'Bot attachment draft does not leak into a project session',
    )
    await cdp.fill(input, 'Project draft stays in project')
    await cdp.evaluate(`window.__botProbe.navigation.openMainTab('bot')`)
    await wait(() => visible('[data-testid="bot-welcome"]'), 'return to Bot')
    check(
      (await active()).draft === 'Bot draft survives project navigation',
      'Bot draft survives project-session navigation',
    )
    check(
      await cdp.evaluate(
        `!![...document.querySelectorAll('button')].find(el=>el.getClientRects().length && el.innerText.includes('Alternate Model'))`,
      ),
      'manually chosen Bot model survives project-session navigation',
    )
    check(
      await cdp.evaluate(`window.__botProbe.chat.getComposerDraft('7')==='Project draft stays in project'`),
      'ordinary session draft stays separate from Bot',
    )
    check(
      await visible('[data-chat-input="true"] [data-attachment-id]'),
      'Bot attachment draft survives a trip to a project conversation',
    )
    // Submit through the same full composer and verify the request's session.
    const priorMessages = requests.filter((r) => r.path.endsWith('/messages')).length
    await cdp.fill(input, 'Fixture Bot message')
    await cdp.send('Input.dispatchKeyEvent', {
      type: 'keyDown',
      key: 'Enter',
      code: 'Enter',
      windowsVirtualKeyCode: 13,
      modifiers: 2,
    })
    await cdp.send('Input.dispatchKeyEvent', {
      type: 'keyUp',
      key: 'Enter',
      code: 'Enter',
      windowsVirtualKeyCode: 13,
      modifiers: 2,
    })
    await wait(() => requests.filter((r) => r.path.endsWith('/messages')).length > priorMessages, 'Bot message request')
    check(
      requests.filter((r) => r.path.endsWith('/messages')).at(-1).path === '/api/v1/sessions/42/messages',
      'full composer sends only to the canonical Bot session',
    )
    await cdp.send('Page.navigate', { url: base + `/bot?botId=${botB}&sessionId=42` })
    await wait(
      () => cdp.evaluate(`!!window.__botProbe && window.__botProbe.activeComposerSession()==='43'`),
      'explicit Muse bookmark reload',
    )
    await wait(
      () => cdp.evaluate(`document.body.innerText.includes('History only for 43')`),
      'Muse persisted history on reload',
    )
    check(
      await cdp.evaluate(
        `new URLSearchParams(location.search).get('sessionId')==='43' && document.querySelector('[data-workspace-pane-window="'+window.__botProbe.ui.activeWorkspaceWindowId+'"] [data-testid="bot-name"]')?.innerText==='Muse'`,
      ),
      'explicit bookmark botId overrides saved tab and forged sessionId after reload',
    )
    await sendCurrent('Muse message after reload', 43)
    await navigate(`/bot?botId=99999999-9999-4999-8999-999999999999`)
    await wait(() => visible('[data-testid="bot-loading"] [role="alert"]'), 'unknown UUID error')
    check(!(await visible(input)), 'unknown explicit UUID cannot use default or stale composer')
    await navigate('/bot?botId=malformed')
    await wait(() => visible('[data-testid="bot-loading"] [role="alert"]'), 'malformed UUID error')
    check(!(await visible(input)), 'malformed explicit UUID is actionable without fallback')
    await chooseBot(botA, 42)
    await cdp.fill(input, '/assistant')
    for (const type of ['keyDown', 'keyUp'])
      await cdp.send('Input.dispatchKeyEvent', {
        type,
        key: 'Enter',
        code: 'Enter',
        windowsVirtualKeyCode: 13,
        modifiers: 2,
      })
    await wait(() => visible('[data-testid="bot-edit-name"]'), 'scoped assistant slash')
    check(
      await cdp.evaluate(
        `location.pathname==='/bot' && document.querySelector('[data-testid="bot-edit-name"]').value==='Atlas'`,
      ),
      '/assistant in Bot opens current persona editor',
    )
    await cdp.key('Escape', 'Escape', 27)
    // Full document navigation must honor a deep link over persisted Bot tabs.
    // Wait for a new document, so the outgoing composer cannot satisfy readiness.
    // The URL can move ahead of the scoped pane; reject the outgoing editor.
    await cdp.fill(input, 'Keep draft while route changes')
    const beforeStaleSend = requests.filter((r) => r.path.endsWith('/messages')).length
    await cdp.evaluate(`history.replaceState(history.state, '', '/chat?sessionId=7')`)
    for (const type of ['keyDown', 'keyUp'])
      await cdp.send('Input.dispatchKeyEvent', {
        type,
        key: 'Enter',
        code: 'Enter',
        windowsVirtualKeyCode: 13,
        modifiers: 2,
      })
    await sleep(200)
    check(
      requests.filter((r) => r.path.endsWith('/messages')).length === beforeStaleSend &&
        (await active()).draft === 'Keep draft while route changes',
      'stale Bot composer refuses a send when the browser URL targets a project and preserves its draft',
    )
    await cdp.evaluate(`history.replaceState(history.state, '', window.__botProbe.router.currentRoute.value.fullPath)`)
    const oldDocument = await cdp.evaluate('performance.timeOrigin')
    await cdp.send('Page.navigate', { url: base + '/chat?sessionId=7' })
    await wait(
      () =>
        cdp.evaluate(
          `performance.timeOrigin!==${oldDocument} && !!window.__botProbe && location.pathname==='/chat' && new URLSearchParams(location.search).get('sessionId')==='7' && !![...document.querySelectorAll('${input}')].find(el=>el.getClientRects().length)`,
        ),
      'explicit project document after persisted Bot',
      90000,
    )
    check(
      (await active()).path === '/chat' && (await active()).session === '7',
      'explicit project deep link wins over persisted active Bot on a new document',
    )
    await cdp.fill(input, 'Project message after persisted Bot')
    const beforeProjectSend = requests.filter((r) => r.path.endsWith('/messages')).length
    // The configured composer uses Ctrl+Enter.
    for (const type of ['keyDown', 'keyUp'])
      await cdp.send('Input.dispatchKeyEvent', {
        type,
        key: 'Enter',
        code: 'Enter',
        windowsVirtualKeyCode: 13,
        modifiers: 2,
      })
    await wait(
      () => requests.filter((r) => r.path.endsWith('/messages')).length > beforeProjectSend,
      'project deep-link send',
    )
    check(
      requests.filter((r) => r.path.endsWith('/messages')).at(-1).path === '/api/v1/sessions/7/messages',
      'new-document explicit project composer sends to project 7, never persisted Bot 42',
    )
    for (const absolute of [false, true]) {
      await cdp.evaluate(`window.__botProbe.navigation.openMainTab('bot')`)
      await wait(() => visible('[data-testid="bot-welcome"]'), 'Bot markdown source')
      await cdp.evaluate(`window.__removeBotLink = window.__botProbe.mountProjectLink(${absolute})`)
      await wait(() => visible('#bot-markdown-link a[href]'), 'regular Markdown app link')
      check(
        await cdp.evaluate(
          `document.querySelector('#bot-markdown-link a').getAttribute('href').includes('/chat?sessionId=7') && !document.querySelector('#bot-markdown-link a').hasAttribute('data-oc-resource-href')`,
        ),
        `${absolute ? 'same-origin absolute' : 'root-relative'} Markdown project link remains an app destination, not session media`,
      )
      await cdp.click('#bot-markdown-link a')
      await wait(
        () =>
          cdp.evaluate(`location.pathname==='/chat' && new URLSearchParams(location.search).get('sessionId')==='7'`),
        'Markdown project target',
      )
      await sleep(200)
      check(
        (await active()).path === '/chat' && (await active()).session === '7',
        `${absolute ? 'same-origin absolute' : 'root-relative'} Markdown link from Bot opens the explicit project without Bot interception`,
      )
      await cdp.evaluate(`window.__removeBotLink()`)
    }
    // Dispatch the shared command through the real command parser/composer.
    await navigate('/chat?sessionId=7')
    await wait(() => cdp.evaluate(`location.pathname==='/chat'`), 'chat command source')
    await cdp.fill(input, '/bot')
    await cdp.send('Input.dispatchKeyEvent', {
      type: 'keyDown',
      key: 'Enter',
      code: 'Enter',
      windowsVirtualKeyCode: 13,
      modifiers: 2,
    })
    await cdp.send('Input.dispatchKeyEvent', {
      type: 'keyUp',
      key: 'Enter',
      code: 'Enter',
      windowsVirtualKeyCode: 13,
      modifiers: 2,
    })
    await wait(
      () => cdp.evaluate(`location.pathname==='/bot' && new URLSearchParams(location.search).get('sessionId')==='42'`),
      'shared bot command',
    )
    check(true, 'shared client command action bot opens the canonical /bot conversation')
    // A deliberately delayed canonical request must not steal a route after leaving.
    await navigate('/integrations?source=7')
    botDelay = 900
    const beforeRace = botRequests
    await navigate('/bot')
    await wait(() => botRequests > beforeRace, 'delayed canonical request')
    await navigate('/chat?sessionId=7')
    await sleep(1100)
    check(
      (await active()).path === '/chat' && (await active()).session === '7',
      'late canonical response cannot steal project session selection',
    )
    botDelay = 0
    botFailures = 1
    await navigate('/integrations?source=7')
    await navigate('/bot')
    await wait(() => visible('[role="alert"]'), 'Bot retry error')
    check(!(await visible(input)), 'failed canonical lookup never exposes another session composer')
    await cdp.evaluate(
      `window.__retryFocusTrace=[]; for(const type of ['focusin','focusout'])document.addEventListener(type,e=>window.__retryFocusTrace.push({type,tag:e.target.tagName,input:e.target.matches?.('[data-chat-input="true"]'),time:performance.now()}),true)`,
    )
    await cdp.click('[data-testid="bot-loading"] [role="alert"] button')
    await wait(() => visible('[data-testid="bot-welcome"]'), 'retry success')
    check((await active()).session === '42', 'accessible retry recovers canonical Bot conversation')
    await wait(
      () => cdp.evaluate(`document.activeElement?.matches('[data-chat-input="true"]')`),
      'retry keyboard focus',
      3000,
    ).catch(async (error) => {
      console.error(
        'FOCUS DEBUG',
        await cdp.evaluate(
          `(() => {let owner=[...document.querySelectorAll('[data-chat-input="true"]')].find(el=>el.getClientRects().length)?.__vueParentComponent;while(owner && !owner.type.__file?.endsWith('/pages/ChatPage.vue'))owner=owner.parent;const state=owner?.setupState;return {active:document.activeElement?.tagName,composer:!!state?.composerRef,botReady:state?.bot?.ready?.value,owns:state?.bot?.ownsConversation?.(),trace:window.__retryFocusTrace,url:location.href}})()`,
        ),
      )
      throw error
    })
    check(true, 'retry restores keyboard focus to the Bot composer')
    await navigate('/assistant')
    await wait(() => visible('[data-testid="bot-edit-name"]'), 'direct assistant shortcut scoped to Bot')
    check(await cdp.evaluate(`location.pathname==='/bot'`), 'assistant route navigation from Bot opens scoped editor')
    await cdp.key('Escape', 'Escape', 27)
    botDelay = 700
    const beforeNamedRace = botRequests
    await navigate(`/bot?botId=${botB}`)
    await wait(() => botRequests > beforeNamedRace, 'delayed Muse ensure')
    await navigate(`/bot?botId=${botA}`)
    await wait(() => cdp.evaluate(`window.__botProbe.activeComposerSession()==='42'`), 'Atlas wins fast switch')
    await sleep(800)
    botDelay = 0
    await sendCurrent('Atlas after fast switching', 42)
    check(
      JSON.stringify(sent.at(-1).body).includes('alternate-model'),
      'Atlas outgoing request retains its own Alternate Model after reload and switching',
    )
    check(
      await cdp.evaluate(`document.querySelector('[data-workspace-pane-window="'+window.__botProbe.ui.activeWorkspaceWindowId+'"] [data-testid="bot-name"]')?.innerText==='Atlas'`),
      'late Muse response cannot relabel Atlas',
    )
    const split = await cdp.evaluate(
      `(() => {const ui=window.__botProbe.ui; const a=ui.activeWorkspaceWindowId;const group=ui.activeWorkspaceGroupId;const b=ui.openWorkspaceWindow('bot',{activate:false,path:'/bot',query:{botId:'${botB}'},title:'Muse',matchKeys:['botId']});ui.insertWorkspaceWindowIntoGroupSplit(group,b,1);return {a,b}})()`,
    )
    await cdp.evaluate(`window.__botProbe.navigation.navigateToWorkspaceWindow('${split.b}')`)
    await wait(
      () =>
        cdp.evaluate(
          `document.querySelector('[data-workspace-pane-window="${split.b}"] [data-chat-input="true"]')?.getClientRects().length && document.querySelector('[data-workspace-pane-window="${split.a}"] [data-chat-input="true"]')?.getClientRects().length`,
        ),
      'independent visible Bot panes',
    )
    check(
      await cdp.evaluate(
        `document.querySelector('[data-workspace-pane-window="${split.a}"] [data-testid="bot-name"]').innerText==='Atlas' && document.querySelector('[data-workspace-pane-window="${split.b}"] [data-testid="bot-name"]').innerText==='Muse'`,
      ),
      'two visible panes retain independent named Bot identities',
    )
    const choosePaneModel = async (windowId, label) => {
      await cdp.evaluate(`window.__botProbe.navigation.navigateToWorkspaceWindow('${windowId}')`)
      const pane = `[data-workspace-pane-window="${windowId}"]`
      const currentLabel = await cdp.evaluate(`(() => {const p=document.querySelector(${JSON.stringify(pane)});return [...p.querySelectorAll('button')].find(n=>n.getClientRects().length && (n.innerText.includes('Fixture Model') || n.innerText.includes('Alternate Model')))?.innerText})()`)
      await cdp.clickText(`${pane} button`, currentLabel)
      await wait(() => visible('[data-option-menu-item]'), 'split model picker')
      await cdp.clickText('[data-option-menu-item]', label)
    }
    await choosePaneModel(split.b, 'Alternate Model')
    await sleep(300)
    await choosePaneModel(split.a, 'Fixture Model')
    await sleep(300)
    check(
      await cdp.evaluate(
        `import('/src/lib/persistence/storageKeys.ts').then(({localStorageKeys})=>{const map=JSON.parse(localStorage.getItem(localStorageKeys.chat.sessionManualModelBySession));return JSON.parse(map['42']).value.includes('fixture-model') && JSON.parse(map['43']).value.includes('alternate-model')})`,
      ),
      'opposing model choices in concurrent panes preserve both session preferences',
    )
    await cdp.evaluate(`window.__botProbe.navigation.navigateToWorkspaceWindow('${split.b}')`)
    const beforeSplitSend = sent.length
    await cdp.fill(`[data-workspace-pane-window="${split.b}"] ${input}`, 'Muse split-pane message')
    for (const type of ['keyDown', 'keyUp'])
      await cdp.send('Input.dispatchKeyEvent', {
        type,
        key: 'Enter',
        code: 'Enter',
        windowsVirtualKeyCode: 13,
        modifiers: 2,
      })
    await wait(() => sent.length > beforeSplitSend, 'focused split Bot send')
    check(
      sent.at(-1).path === '/api/v1/sessions/43/messages',
      'focused Muse pane sends only to Muse while Atlas remains open',
    )
    check(
      JSON.stringify(sent.at(-1).body).includes('alternate-model'),
      'Muse split pane sends its own Alternate Model while Atlas uses Fixture Model',
    )
    await cdp.screenshot(path.join(evidence, 'web-bot-two-panes.png'))
    await cdp.evaluate(
      `window.__botProbe.ui.closeWorkspaceWindow('${split.b}');window.__botProbe.navigation.navigateToWorkspaceWindow('${split.a}')`,
    )
    await wait(() => cdp.evaluate(`window.__botProbe.activeComposerSession()==='42'`), 'close split')
    // Narrow touch viewport, same real responsive shell and sidebar.
    await cdp.send('Emulation.setDeviceMetricsOverride', {
      width: 375,
      height: 812,
      deviceScaleFactor: 1,
      mobile: true,
    })
    await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 1 })
    await cdp.send('Page.navigate', { url: base + '/bot' })
    await wait(() => visible('.oc-bottom-nav a[href="/bot"]'), 'mobile Bot navigation')
    await wait(() => visible('[data-testid="bot-welcome"]'), 'mobile canonical conversation')
    check(
      await visible('.oc-bottom-nav a[href="/bot"]'),
      'Bot is directly accessible in mobile primary navigation at 375px',
    )
    check(
      await cdp.evaluate(
        `document.documentElement.scrollWidth<=innerWidth && document.querySelector('[data-testid="bot-welcome"]').getBoundingClientRect().right<=innerWidth+1`,
      ),
      '375px Bot layout has no horizontal overflow',
    )
    await cdp.click('[data-testid="bot-selector"]')
    await wait(() => visible('[data-testid="bot-manager"]'), 'mobile named manager')
    check(
      await cdp.evaluate(
        `document.querySelector('[role="dialog"]').getBoundingClientRect().right<=innerWidth+1 && document.documentElement.scrollWidth<=innerWidth`,
      ),
      '375px named Bot manager fits viewport',
    )
    await cdp.screenshot(path.join(evidence, 'web-bot-manager-mobile.png'))
    await cdp.key('Escape', 'Escape', 27)
    await chooseBot(botB, 43)
    await sendCurrent('Muse message on mobile', 43)
    check(
      JSON.stringify(sent.at(-1).body).includes('alternate-model'),
      'Muse independent model survives document reload after opposing pane choices',
    )
    await cdp.click('[data-testid="bot-settings"]')
    await wait(() => visible('[data-testid="bot-edit-instructions"]'), 'mobile Bot persona editor')
    check(
      await cdp.evaluate(
        `document.documentElement.scrollWidth<=innerWidth && !document.querySelector('[role="dialog"]').innerText.includes('memory_enabled')`,
      ),
      '375px persona editor fits viewport and contains only Bot persona controls',
    )
    await cdp.screenshot(path.join(evidence, 'web-bot-persona-mobile.png'))
    await cdp.key('Escape', 'Escape', 27)
    await chooseBot(botA, 42)
    await cdp.click('[data-bot-suggestion="create"]')
    const createSuggestion = await cdp.evaluate(
      `import('/src/i18n/index.ts').then(({ i18n }) => i18n.global.t('bot.prompts.create'))`,
    )
    check((await active()).draft.includes(createSuggestion), 'mobile starting suggestion fills the canonical Bot draft')
    await cdp.evaluate(`document.activeElement?.blur()`)
    await cdp.screenshot(path.join(evidence, 'web-bot-mobile.png'))
    await cdp.evaluate(`window.__botProbe.ui.setSessionSwitcherOpen(true)`)
    await wait(() => visible('[data-testid="sidebar-bot"]'), 'mobile session switcher')
    check(await visible('[data-testid="sidebar-bot"]') && await visible('[data-testid="bot-sidebar"]'), 'mobile Bot drawer uses the standalone sidebar')
    await wait(()=>visible(`${row(botB)} [data-testid="sidebar-bot-actions"]`), 'mobile sidebar row menu')
    const mobileRoute=await cdp.evaluate('location.pathname+location.search')
    await cdp.click(`${row(botB)} [data-testid="sidebar-bot-actions"]`)
    await cdp.clickText('[data-option-menu-item]', 'Edit Bot')
    await wait(()=>visible('[data-testid="bot-edit-name"]'), 'mobile touch opens row editor')
    check(await cdp.evaluate(`document.documentElement.scrollWidth<=innerWidth+1 && [...document.querySelectorAll('[role="dialog"]')].filter(n=>n.getClientRects().length).every(n=>n.getBoundingClientRect().right<=innerWidth+1)`), '375px touch row menu opens a fitting shared editor')
    check(await cdp.evaluate('location.pathname+location.search')===mobileRoute, 'mobile row action does not open the unselected Bot')
    await cdp.key('Escape','Escape',27)
    await wait(async()=>!(await visible('[data-testid="bot-persona-form"]')), 'close mobile editor')
    await cdp.evaluate(`window.__botProbe.ui.setSessionSwitcherOpen(true)`)
    await wait(()=>visible('[data-testid="sidebar-bots"]'), 'mobile project drawer returns')
    const directoriesTitle = await cdp.evaluate(`import('/src/i18n/index.ts').then(({i18n})=>i18n.global.t('chat.sidebar.header.directoriesTitle'))`)
    check(await cdp.evaluate(`![...document.querySelectorAll('p')].some(n=>n.getClientRects().length && n.textContent.trim()===${JSON.stringify(directoriesTitle)})`), 'mobile Bot drawer excludes project navigation')
    await cdp.screenshot(path.join(evidence, 'web-bot-mobile-projects.png'))
    await cdp.click('[data-testid="sidebar-bot"]')
    await wait(() => cdp.evaluate(`!window.__botProbe.ui.isSessionSwitcherOpen`), 'switcher closes on Bot entry')
    check(true, 'mobile Bot entry closes the session switcher and keeps chat usable')
    check(
      await cdp.evaluate(
        `!document.querySelector('[data-main-nav-tab="tasks"], [data-testid="chat-tasks"], a[href^="/tasks"]')`,
      ),
      'Task is absent from mobile navigation and session switcher at 375px',
    )
    const sendBot = async (text) => {
      const count = botActions.length
      await cdp.fill(input, text)
      for (const type of ['keyDown', 'keyUp'])
        await cdp.send('Input.dispatchKeyEvent', {
          type,
          key: 'Enter',
          code: 'Enter',
          windowsVirtualKeyCode: 13,
          modifiers: 2,
        })
      await wait(() => botActions.length > count, 'Bot fixture tool action')
    }
    await sendBot('Create a regular project session')
    await wait(() => visible('a[href="/chat?sessionId=8"]'), 'Bot creation result link')
    await cdp.fill(input, 'Bot draft while opening created session')
    await cdp.click('a[href="/chat?sessionId=8"]')
    await wait(
      () => cdp.evaluate(`location.pathname==='/chat' && window.__botProbe.activeComposerSession()==='8'`),
      'created ordinary session',
    )
    await wait(() => visible(input), 'created session composer')
    await cdp.fill(input, 'Created session draft')
    check(
      await cdp.evaluate(`window.__botProbe.chat.getComposerDraft('42')==='Bot draft while opening created session'`),
      'Bot creation result opens ordinary session 8 and preserves the Bot draft',
    )
    await cdp.evaluate(`window.__botProbe.navigation.openMainTab('bot')`)
    await wait(
      () => cdp.evaluate(`location.pathname==='/bot' && window.__botProbe.activeComposerSession()==='42'`),
      'return from creation',
    )
    await sendBot('List project sessions')
    await wait(() => visible('a[href="/chat?sessionId=7"]'), 'Bot session list result')
    await cdp.fill(input, 'Bot draft while opening listed session')
    await cdp.click('a[href="/chat?sessionId=7"]')
    await wait(
      () => cdp.evaluate(`location.pathname==='/chat' && window.__botProbe.activeComposerSession()==='7'`),
      'listed ordinary session',
    )
    check(
      await cdp.evaluate(
        `window.__botProbe.chat.getComposerDraft('8')==='Created session draft' && window.__botProbe.chat.getComposerDraft('42')==='Bot draft while opening listed session'`,
      ),
      'Bot list links open the explicit ordinary session and retain separate Bot/created-session drafts',
    )
    check(
      botActions.every((action) => action.tool === 'bot.sessions'),
      'Bot creation/list fixtures use bot.sessions',
    )
    check(
      await cdp.evaluate(`document.documentElement.scrollWidth<=innerWidth+1`),
      '375px ordinary chat after Bot links has no overflow',
    )
    await cdp.screenshot(path.join(evidence, 'web-bot-session-link-mobile.png'))
    await navigate('/integrations?source=7')
    await wait(() => visible('[data-provider="example.source"]'), 'native integration provider')
    await cdp.click('[data-provider="example.source"]')
    const proposal = await cdp.evaluate(`JSON.parse(document.querySelector('[data-testid="integration-json"]').value)`)
    check(
      proposal.max_execution_seconds === 300 && !('max_task_seconds' in proposal),
      'integration proposal uses max_execution_seconds',
    )
    proposal.mode = 'session'
    await cdp.fill('[data-testid="integration-json"]', JSON.stringify(proposal))
    await cdp.click('[data-testid="integration-create"]')
    await wait(() => (!cdp ? false : visible('[data-testid="integration-enable"]')), 'saved integration draft')
    await wait(
      () => bindings.length === 2 && !bindings[1].state.includes('enabled'),
      'disabled session proposal persisted',
    )
    await wait(() => cdp.evaluate(`!document.querySelector('[role="dialog"]')`), 'integration draft dialog closes')
    await cdp.click('[data-testid="integration-enable"]')
    // The mobile keyboard dismissal guard can consume the first real touch.
    // A human taps once more to open the review; do the same only if needed.
    if (!(await visible('[data-testid="integration-confirm"]'))) {
      await cdp.click('[data-testid="integration-enable"]')
    }
    await wait(() => visible('[data-testid="integration-confirm"]'), 'enable review dialog opened')
    check(bindings[1].state === 'disabled', 'opening integration review does not enable session execution')
    await cdp.click('[data-testid="integration-confirm"]')
    await wait(() => bindings[1].state === 'enabled', 'enable integration')
    await wait(() => cdp.evaluate(`!document.querySelector('[role="dialog"]')`), 'enable dialog closes')
    await cdp.click('[data-testid="integration-pause"]')
    if (!(await visible('[data-testid="integration-confirm"]'))) {
      await cdp.click('[data-testid="integration-pause"]')
    }
    await wait(() => visible('[data-testid="integration-confirm"]'), 'pause review dialog opened')
    await cdp.click('[data-testid="integration-confirm"]')
    await wait(() => bindings[1].state === 'paused', 'pause integration')
    await wait(() => cdp.evaluate(`!document.querySelector('[role="dialog"]')`), 'pause dialog closes')
    check(
      bindings[1].spec.mode === 'session',
      'native integration create, enable and pause preserve session execution mode',
    )
    await cdp.click('[data-binding="channel-fixture"]')
    await wait(() => visible('[data-testid="channel-access-edit"]'), 'channel role management')
    await cdp.click('[data-testid="channel-access-edit"]')
    await cdp.click('[role="dialog"] details > summary')
    await cdp.fill('[data-testid="channel-tools-owner"]', 'Subagent')
    await cdp.fill('[data-testid="channel-mcp-owner"]', '{"docs":["search"]}')
    check(
      await cdp.evaluate(`!document.querySelector('[role="dialog"]').innerText.includes('后台工作')`),
      'channel editor has no retired background-work capability',
    )
    await cdp.click('[data-testid="channel-access-save"]')
    await wait(() => accessRequests.length === 1, 'explicit channel grant saved')
    check(
      accessRequests[0].policy.grants.owner.tools.includes('Subagent') &&
        accessRequests[0].policy.grants.owner.mcp_tools.docs[0] === 'search' &&
        !JSON.stringify(accessRequests[0]).includes('background_work'),
      'channel roles preserve explicit Subagent and MCP grants without background_work',
    )
    await wait(() => cdp.evaluate(`!document.querySelector('[role="dialog"]')`), 'channel dialog closes')
    check(
      await cdp.evaluate(`document.documentElement.scrollWidth<=innerWidth+1`),
      '375px integration and channel management has no overflow',
    )
    await cdp.screenshot(path.join(evidence, 'web-integrations-mobile.png'))
    check(
      await cdp.evaluate(
        `document.querySelector('[data-testid="integration-event-session"]').getAttribute('href')==='/chat?sessionId=8'`,
      ),
      'integration event exposes a normal session chat link',
    )
    await cdp.click('[data-testid="integration-event-session"]')
    await wait(
      () =>
        cdp.evaluate(
          `location.pathname==='/chat' && new URLSearchParams(location.search).get('sessionId')==='8' && window.__botProbe.activeComposerSession()==='8'`,
        ),
      'integration event session link',
    )
    check(true, 'integration event execution opens normal /chat?sessionId=8 with its exact session')
    check(
      !requests.some((r) => /^\/api\/v1\/(tasks(?:\/|$)|chatbot\/messages$)/.test(r.path)),
      'no browser flow ever requests retired Task or chatbot endpoints, including reads',
    )
    check(
      !requests.some((r) => r.method === 'POST' && r.path === '/api/v1/sessions'),
      'frontend resolves Bot through its canonical endpoint and creates ordinary sessions only through Bot tools',
    )
    await navigate(`/bot?botId=${botA}`)
    await wait(() => cdp.evaluate(`window.__botProbe.activeComposerSession()==='42'`), 'all archived test source')
    for (const id of [botB, botA]) {
      await cdp.click('[data-testid="bot-selector"]')
      await cdp.click(`[data-bot-archive="${id}"]`)
      await cdp.click('[data-testid="bot-archive-confirm"]')
      await wait(() => profiles.find((bot) => bot.id === id).archived, 'archive named Bot')
      await wait(async () => !(await visible('[data-testid="bot-archive-confirm"]')), 'archive confirmation closes')
      await cdp.key('Escape', 'Escape', 27)
      await wait(async () => !(await visible('[data-testid="bot-manager"]')), 'manager closes after archive')
    }
    await navigate('/bot')
    await wait(() => visible('[data-testid="bot-loading"] [role="alert"]'), 'no active default error')
    await cdp.click('[data-testid="bot-selector"]')
    await wait(
      () =>
        cdp.evaluate(
          `document.querySelector('[data-testid="bot-manager"]') && !document.querySelector('[data-bot-select]')`,
        ),
      'empty Bot registry create state',
    )
    check(
      (await visible('[data-testid="bot-create"]')) && !(await visible(input)),
      'all archived state offers explicit creation with no silent resurrection',
    )
    check(
      !requests.some((r) => r.method === 'DELETE' && /^\/api\/v1\/sessions/.test(r.path)),
      'archival never deletes physical sessions',
    )
    await cdp.screenshot(path.join(evidence, 'web-bot-empty-mobile.png'))
    check(errors.length === 0, `no browser exceptions (${errors.length})`)
    await fs.writeFile(
      path.join(evidence, 'web-browser-report.json'),
      JSON.stringify({ checks, requests, profileRequests, sent, errors }, null, 2),
    )
  } catch (error) {
    if (cdp) {
      console.error('KEYBOARD TRACE', await cdp.evaluate('JSON.stringify(window.__botKeyboardTrace)').catch(() => ''))
      await cdp.screenshot(path.join(evidence, 'web-browser-failure.png')).catch(() => {})
      await fs.writeFile(
        path.join(evidence, 'web-browser-failure.txt'),
        await cdp.evaluate('document.body.innerText').catch(() => ''),
      )
    }
    await fs.writeFile(
      path.join(evidence, 'web-browser-report.json'),
      JSON.stringify({ checks, requests, profileRequests, sent, errors }, null, 2),
    )
    throw error
  } finally {
    cdp?.close()
    if (chrome && chrome.exitCode === null) {
      chrome.kill('SIGTERM')
      await Promise.race([new Promise((resolve) => chrome.once('exit', resolve)), sleep(3000)])
      if (chrome.exitCode === null) chrome.kill('SIGKILL')
    }
    server?.httpServer?.closeAllConnections()
    await server?.close()
    await fs.rm(temporary, { recursive: true, force: true })
    await fs.rm(profile, { recursive: true, force: true })
  }
}
main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
