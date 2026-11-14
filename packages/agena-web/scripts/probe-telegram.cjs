// Real authenticated Agena + native Chrome CDP + two isolated local Telegram accounts.
// Requires Node and Chrome; no Playwright package or external provider is used.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const os = require('node:os')
const http = require('node:http')
const net = require('node:net')
const crypto = require('node:crypto')
const { spawn } = require('node:child_process')
const { once } = require('node:events')

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
async function until(read, predicate, title, timeout = 45000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) {
    const result = await read()
    if (predicate(result)) return result
    await sleep(150)
  }
  throw new Error('Timed out: ' + title)
}
async function freePort() {
  const server = net.createServer()
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  const port = server.address().port
  await new Promise((resolve) => server.close(resolve))
  return port
}
async function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  const done = once(child, 'exit').catch(() => {})
  child.kill('SIGTERM')
  const timer = setTimeout(() => child.kill('SIGKILL'), 8000)
  await done
  clearTimeout(timer)
}

class Chrome {
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
        reject(new Error('CDP timeout: ' + method))
      }, 30000)
      this.pending.set(id, { resolve, reject, timer })
      this.socket.send(JSON.stringify({ id, method, params }))
    })
  }
  async evaluate(expression) {
    const result = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true })
    if (result.exceptionDetails)
      throw new Error(result.exceptionDetails.exception?.description || JSON.stringify(result.exceptionDetails))
    return result.result?.value
  }
  visible(selector) {
    return this.evaluate(
      '!![...document.querySelectorAll(' +
        JSON.stringify(selector) +
        ")].find(n=>n.getClientRects().length && getComputedStyle(n).visibility !== 'hidden')",
    )
  }
  waitVisible(selector, visible = true, timeout = 45000) {
    return until(
      () => this.visible(selector),
      (value) => value === visible,
      selector + (visible ? ' visible' : ' hidden'),
      timeout,
    )
  }
  async click(selector) {
    const point = await until(
      () =>
        this.evaluate(
          '(() => {const n=[...document.querySelectorAll(' +
            JSON.stringify(selector) +
            ")].find(n=>n.getClientRects().length && !n.disabled);if(!n)return null;n.scrollIntoView({block:'center',behavior:'instant'});const r=n.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2;return n.contains(document.elementFromPoint(x,y))?{x,y}:null})()",
        ),
      Boolean,
      'click ' + selector,
    )
    await this.send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...point })
    for (const type of ['mousePressed', 'mouseReleased'])
      await this.send('Input.dispatchMouseEvent', { type, ...point, button: 'left', clickCount: 1 })
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
  }
  async select(selector, value) {
    await until(
      () =>
        this.evaluate(
          '(() => {const n=[...document.querySelectorAll(' +
            JSON.stringify(selector) +
            ')].find(n=>n.getClientRects().length && !n.disabled);if(!n || ![...n.options].some(o=>o.value===' +
            JSON.stringify(value) +
            '))return false;n.value=' +
            JSON.stringify(value) +
            ";n.dispatchEvent(new Event('input',{bubbles:true}));n.dispatchEvent(new Event('change',{bubbles:true}));return true})()",
        ),
      Boolean,
      'select ' + selector,
    )
  }
  async check(selector, checked = true) {
    await this.waitVisible(selector)
    if ((await this.evaluate('document.querySelector(' + JSON.stringify(selector) + ').checked')) !== checked)
      await this.click(selector)
    await until(
      () => this.evaluate('document.querySelector(' + JSON.stringify(selector) + ')?.checked'),
      (value) => value === checked,
      'checkbox ' + selector,
    )
  }
  async key(key, code, number, text) {
    await this.send('Input.dispatchKeyEvent', {
      type: 'keyDown',
      key,
      code,
      windowsVirtualKeyCode: number,
      ...(text ? { text } : {}),
    })
    await this.send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode: number })
  }
  async goto(url) {
    let timer
    const loaded = new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error('Page load timeout')), 90000)
      this.events.set('Page.loadEventFired', resolve)
    })
    try {
      await Promise.all([this.send('Page.navigate', { url }), loaded])
    } finally {
      clearTimeout(timer)
      this.events.delete('Page.loadEventFired')
    }
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
  assert(!process.versions.bun, 'Run this browser probe with Node')
  const repo = path.resolve(__dirname, '../../..')
  const tmp = await fs.mkdtemp(path.join(process.env.TELEGRAM_EVIDENCE_ROOT || os.tmpdir(), 'agena-telegram-probe-'))
  const data = path.join(tmp, 'data')
  const workspace = path.join(tmp, 'workspace')
  const chromeProfile = path.join(tmp, 'chrome')
  await Promise.all([data, workspace, chromeProfile].map((directory) => fs.mkdir(directory, { recursive: true })))
  await fs.writeFile(
    path.join(data, 'agena.json'),
    JSON.stringify({ runtime: { providers: { client_versions: { auto_update: false } } } }),
  )
  const accounts = new Map([
    ['123:fixture-work', { id: 123, username: 'work_bot', updates: [], offsets: [], sent: [], lookups: [] }],
    ['456:fixture-home', { id: 456, username: 'home_bot', updates: [], offsets: [], sent: [], lookups: [] }],
  ])
  const mock = http.createServer(async (request, response) => {
    const chunks = []
    for await (const chunk of request) chunks.push(chunk)
    const body = chunks.length ? JSON.parse(Buffer.concat(chunks)) : {}
    const parts = request.url.split('/')
    const bot = accounts.get(parts[1]?.slice(3))
    const method = parts[2]
    const json = (status, payload) =>
      response.writeHead(status, { 'content-type': 'application/json' }).end(JSON.stringify(payload))
    if (!bot) return json(401, { ok: false, error_code: 401, description: 'Unauthorized' })
    let result
    if (method === 'getMe') {
      result = {
        id: bot.id,
        username: bot.username,
        is_bot: true,
        first_name: bot.username,
        can_join_groups: true,
        can_read_all_group_messages: false,
      }
    } else if (method === 'getWebhookInfo') {
      result = { url: '', pending_update_count: bot.updates.length }
    } else if (method === 'getUpdates') {
      bot.offsets.push(body.offset ?? null)
      if (body.offset) bot.updates = bot.updates.filter((update) => update.update_id >= body.offset)
      result = bot.updates.slice(0, body.limit || 100)
    } else if (method === 'getChat') {
      bot.lookups.push(body.chat_id)
      const id = body.chat_id === '@public_team' ? -100777 : Number(body.chat_id)
      if (!Number.isSafeInteger(id) || id === 0)
        return json(400, { ok: false, error_code: 400, description: 'Unknown fixture chat' })
      result = {
        id,
        type: id > 0 ? 'private' : id === -100456 ? 'channel' : 'supergroup',
        title: id > 0 ? 'Alice' : id === -100456 ? 'Announcements' : id === -100777 ? 'Public team' : 'Team space',
        username: id > 0 ? 'alice' : id === -100777 ? 'public_team' : 'team_space',
        description: 'Local Telegram test chat',
      }
    } else if (method === 'getChatMember') {
      result = {
        status: 'administrator',
        user: { id: bot.id, is_bot: true },
        can_post_messages: true,
        can_manage_topics: true,
      }
    } else if (method === 'sendMessage') {
      bot.sent.push(body)
      result = { message_id: 999 }
    } else return json(404, { ok: false, error_code: 404, description: 'Unknown method' })
    json(200, { ok: true, result })
  })
  await new Promise((resolve) => mock.listen(0, '127.0.0.1', resolve))
  const botBase = 'http://127.0.0.1:' + mock.address().port
  const port = await freePort()
  const origin = 'http://127.0.0.1:' + port
  const password = crypto.randomBytes(24).toString('hex')
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('AGENA_')))
  Object.assign(env, { AGENA_DATA_DIR: data, AGENA_SERVER_PASSWORD: password, AGENA_MCP_ENABLED: 'false' })
  const logfile = await fs.open(path.join(tmp, 'server.log'), 'a')
  const checks = []
  const errors = []
  const httpErrors = []
  let server, chrome, browser, token, failure
  const pass = (name) => {
    checks.push({ name, passed: true })
    console.log('PASS ' + name)
  }
  async function request(method, url, body, authenticated = true) {
    return fetch(origin + url, {
      method,
      headers: {
        ...(body === undefined ? {} : { 'content-type': 'application/json' }),
        ...(authenticated && token ? { authorization: 'Bearer ' + token } : {}),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
  }
  async function api(method, url, body) {
    const response = await request(method, url, body)
    const text = await response.text()
    assert.ok(response.ok, method + ' ' + url + ': ' + response.status + ' ' + text)
    return text ? JSON.parse(text) : null
  }
  async function command(name, input = {}) {
    const response = await api('POST', '/api/v1/plugins/agena.telegram/commands/' + name + '/invoke', {
      input,
      session_id: null,
      slash: null,
      raw: '',
    })
    assert.equal(response.result.status, 'succeeded', JSON.stringify(response))
    return response.result.output
  }
  const catalogBot = async (id) => (await command('list_bots')).bots.find((bot) => bot.id === id)
  const integrations = async (source, telegramId) =>
    (await api('GET', '/api/v1/sessions/' + source + '/integrations')).bindings.filter(
      (binding) =>
        binding.source_session_id === source &&
        binding.spec.provider === 'agena.telegram' &&
        binding.spec.config.bot_id === telegramId,
    )
  async function waitBinding(source, telegramId, chatId, state = 'enabled') {
    return until(
      () => integrations(source, telegramId),
      (bindings) =>
        bindings.some((binding) => String(binding.spec.resource) === String(chatId) && binding.state === state),
      'chat ' + chatId + ' binding ' + state,
    ).then((bindings) =>
      bindings.find((binding) => String(binding.spec.resource) === String(chatId) && binding.state === state),
    )
  }
  async function start() {
    server = spawn(
      process.env.AGENA_BIN || path.join(repo, 'target/debug/agena'),
      [
        '--data-dir',
        data,
        'server',
        '--host',
        '127.0.0.1',
        '--port',
        String(port),
        '--workspace',
        workspace,
        '--ui-dir',
        process.env.AGENA_WEB_UI_DIR || path.join(repo, 'packages/agena-web/dist'),
        '--mcp-enabled',
        'false',
      ],
      { cwd: workspace, env, stdio: ['ignore', logfile.fd, logfile.fd] },
    )
    await until(
      async () => {
        if (server.exitCode !== null) throw new Error('Server exited; see isolated server.log')
        return request('GET', '/auth/session', undefined, false)
          .then((response) => [200, 401].includes(response.status))
          .catch(() => false)
      },
      Boolean,
      'isolated server',
      120000,
    )
    token = (await api('POST', '/auth/session', { password })).token
  }
  async function startBrowser() {
    chrome = spawn(
      process.env.CHROME_BIN || '/usr/bin/google-chrome',
      [
        '--headless=new',
        '--no-sandbox',
        '--disable-dev-shm-usage',
        '--disable-background-networking',
        '--disable-extensions',
        '--remote-debugging-port=0',
        '--user-data-dir=' + chromeProfile,
        'about:blank',
      ],
      { stdio: ['ignore', 'ignore', 'ignore'] },
    )
    const debuggingPort = await until(
      async () => {
        if (chrome.exitCode !== null) throw new Error('Chrome exited before startup')
        return fs
          .readFile(path.join(chromeProfile, 'DevToolsActivePort'), 'utf8')
          .then((text) => Number(text.split('\n')[0]))
          .catch(() => 0)
      },
      Boolean,
      'Chrome startup',
    )
    const target = await (
      await fetch('http://127.0.0.1:' + debuggingPort + '/json/new?about:blank', { method: 'PUT' })
    ).json()
    const socket = new WebSocket(target.webSocketDebuggerUrl)
    await new Promise((resolve, reject) => {
      socket.addEventListener('open', resolve, { once: true })
      socket.addEventListener('error', reject, { once: true })
    })
    browser = new Chrome(socket)
    browser.events.set('Runtime.exceptionThrown', (event) =>
      errors.push(event.exceptionDetails.exception?.description || JSON.stringify(event)),
    )
    browser.events.set('Network.responseReceived', ({ response }) => {
      const url = new URL(response.url)
      if (url.origin === origin && response.status >= 400)
        httpErrors.push({ path: url.pathname, status: response.status })
    })
    browser.events.set('Fetch.requestPaused', (event) => {
      const url = new URL(event.request.url)
      const allowed = url.origin === origin || ['data:', 'blob:'].includes(url.protocol)
      void browser
        .send(allowed ? 'Fetch.continueRequest' : 'Fetch.failRequest', {
          requestId: event.requestId,
          ...(!allowed ? { errorReason: 'BlockedByClient' } : {}),
        })
        .catch(() => {})
    })
    await browser.send('Runtime.enable')
    await browser.send('Network.enable')
    await browser.send('Page.enable')
    await browser.send('Page.bringToFront')
    await browser.send('Fetch.enable', { patterns: [{ urlPattern: '*' }] })
    await browser.send('Emulation.setDeviceMetricsOverride', {
      width: 1440,
      height: 1000,
      deviceScaleFactor: 1,
      mobile: false,
    })
    await browser.send('Page.addScriptToEvaluateOnNewDocument', {
      source: "localStorage.setItem('studio.ui.locale', 'en-US')",
    })
  }
  async function login() {
    await until(
      async () => (await browser.visible('#password')) || (await browser.visible('[data-testid="telegram-manager"]')),
      Boolean,
      'login or Telegram manager',
    )
    if (await browser.visible('#password')) {
      await browser.fill('#password', password)
      await browser.key('Enter', 'Enter', 13, '\r')
    }
    await browser.waitVisible('[data-testid="telegram-manager"]')
  }
  async function openBot(id, view = 'chats') {
    await browser.goto(origin + '/plugins/agena.telegram/manage?bot=' + id + '&view=' + view)
    await login()
    await browser.waitVisible('[data-telegram-bot="' + id + '"]')
    await until(
      () =>
        browser.evaluate(
          'document.querySelector(' +
            JSON.stringify('[data-telegram-bot="' + id + '"]') +
            ')?.getAttribute("aria-pressed")',
        ),
      (value) => value === 'true',
      'selected Telegram bot',
    )
  }
  async function addBot(name, botToken, aiBotId) {
    await browser.click('[data-testid="telegram-add-bot"]')
    await browser.fill('[data-testid="telegram-bot-name"]', name)
    await browser.fill('[data-testid="telegram-token"]', botToken)
    await browser.select('[data-testid="telegram-bot-ai"]', aiBotId)
    await browser.click('[data-testid="telegram-bot-editor"] summary')
    await browser.fill('[data-testid="telegram-api-base"]', botBase)
    await browser.check('[data-testid="telegram-loopback"]')
    await browser.click('[data-testid="telegram-save"]')
    await browser.waitVisible('[data-testid="telegram-bot-editor"]', false)
    const bot = (await command('list_bots')).bots.find((entry) => entry.name === name)
    assert.ok(bot, 'saved Telegram bot')
    assert.equal(bot.ai_bot_id, aiBotId)
    assert.ok(bot.source_session_id > 0)
    return bot
  }
  async function saveAccess(id, predicate) {
    const before = (await command('list_bots')).revision
    await browser.click('[data-testid="telegram-save-access"]')
    const saved = await until(
      () => command('list_bots'),
      (catalog) => catalog.revision > before && predicate(catalog.bots.find((bot) => bot.id === id)),
      'saved chat access',
    )
    await until(
      () => browser.evaluate('document.querySelector("[data-testid=telegram-save-access]")?.disabled'),
      Boolean,
      'saved policy UI settles',
    )
    return saved.bots.find((bot) => bot.id === id)
  }
  async function manualChat(id, kind, title) {
    await browser.click('[data-testid="telegram-add-chat"]')
    await browser.fill('[data-testid="telegram-manual-id"]', String(id))
    await browser.select('[data-testid="telegram-manual-kind"]', kind)
    if (title) await browser.fill('[data-testid="telegram-manual-name"]', title)
    await browser.click('[data-testid="telegram-manual-save"]')
    await browser.waitVisible('[data-testid="telegram-manual-chat"]', false)
  }
  function seed(botToken, id, chat, type, text) {
    const bot = accounts.get(botToken)
    const key = type === 'channel' ? 'channel_post' : 'message'
    bot.updates.push({
      update_id: id,
      [key]: {
        message_id: id,
        // Discovery exercises real polling without asking a model to run.
        date: Math.floor(Date.now() / 1000) - 120,
        chat: {
          id: chat,
          type,
          title:
            type === 'private'
              ? 'Alice'
              : type === 'channel'
                ? 'Announcements'
                : chat === -100999
                  ? 'Future team space'
                  : 'Team space',
          username: type === 'private' ? 'alice' : undefined,
        },
        ...(type === 'channel'
          ? {}
          : { from: { id: type === 'private' ? chat : 77, first_name: 'Alice', is_bot: false } }),
        text,
        ...(type === 'supergroup' ? { message_thread_id: 42, forum_topic_created: { name: 'Engineering' } } : {}),
      },
    })
  }
  const chatCheck = (id) => '[data-telegram-chat-check="' + id + '"]'
  const noOverflow = () =>
    browser.evaluate(
      'document.documentElement.scrollWidth <= innerWidth + 1 && [...document.querySelectorAll("[role=dialog]")].filter(n=>n.getClientRects().length).every(n=>n.getBoundingClientRect().left>=-1 && n.getBoundingClientRect().right<=innerWidth+1)',
    )
  try {
    await start()
    assert.equal(
      (await request('POST', '/api/v1/plugins/agena.telegram/commands/list_bots/invoke', { input: {} }, false)).status,
      401,
    )
    const atlas = await api('POST', '/api/v1/bots', {
      request_key: crypto.randomUUID(),
      name: 'Atlas',
      persona: { instructions: 'Work assistant fixture.' },
    })
    const muse = await api('POST', '/api/v1/bots', {
      request_key: crypto.randomUUID(),
      name: 'Muse',
      persona: { instructions: 'Home assistant fixture.' },
    })
    const aiChoices = await command('list_ai_bots')
    assert.ok(aiChoices.bots.some((bot) => bot.id === atlas.id) && aiChoices.bots.some((bot) => bot.id === muse.id))
    pass('authenticated plugin management uses the same two named AI Bots as the application')
    await startBrowser()
    await browser.goto(origin + '/plugins/agena.telegram/manage')
    await login()
    const work = await addBot('Work Telegram', '123:fixture-work', atlas.id)
    const home = await addBot('Home Telegram', '456:fixture-home', muse.id)
    assert.equal(work.connection.account, 'work_bot')
    assert.equal(home.connection.account, 'home_bot')
    assert.equal((await command('check_connection', { bot_id: work.id })).bot_id, '123')
    assert.equal((await command('check_connection', { bot_id: home.id })).bot_id, '456')
    assert.notEqual(work.source_session_id, home.source_session_id)
    assert.equal((await api('GET', '/api/v1/bots/' + atlas.id)).local_session_id, work.source_session_id)
    assert.equal((await api('GET', '/api/v1/bots/' + muse.id)).local_session_id, home.source_session_id)
    assert.equal((await api('GET', '/api/v1/sessions/' + work.source_session_id)).owner_bot_id, atlas.id)
    const catalog = await command('list_bots')
    const wire =
      JSON.stringify(catalog) + (await browser.evaluate('document.body.innerText + JSON.stringify(localStorage)'))
    assert.ok(!wire.includes('fixture-work') && !wire.includes('fixture-home'))
    pass('Web creation needs name, token and AI Bot; getMe supplies usernames and each AI owns one local source')
    pass('saved tokens stay out of returned catalogs, browser text and local storage')

    const stale = await request('POST', '/api/v1/plugins/agena.telegram/commands/save_bot/invoke', {
      input: {
        expected_revision: 0,
        bot: {
          id: work.id,
          name: 'stale overwrite',
          enabled: true,
          credential_kind: 'vault',
          connection: work.connection,
          ai_bot_id: atlas.id,
          chat_access: work.chat_access,
        },
        token: null,
      },
      session_id: null,
      slash: null,
      raw: '',
    })
    assert.ok(!stale.ok || (await stale.json()).result.status !== 'succeeded')
    assert.equal((await catalogBot(work.id)).name, 'Work Telegram')
    pass('stale policy revisions cannot overwrite a saved Telegram bot')

    seed('123:fixture-work', 1, 77, 'private', 'work private message')
    seed('123:fixture-work', 2, -100123, 'supergroup', 'work group message')
    seed('123:fixture-work', 3, -100456, 'channel', 'work channel message')
    seed('456:fixture-home', 1, 77, 'private', 'home private message')
    const workInbox = await until(
      () => command('bot_inventory', { bot_id: work.id }),
      (value) => value.rooms.length === 3,
      'background work discovery without any binding',
    )
    await until(
      () => command('bot_inventory', { bot_id: home.id }),
      (value) => value.rooms.length === 1,
      'background home discovery without any binding',
    )
    assert.equal((await integrations(work.source_session_id, work.id)).length, 0)
    assert.equal((await integrations(home.source_session_id, home.id)).length, 0)
    assert.equal(workInbox.rooms.find((room) => room.chat_id === '-100123').topics['42'], 'Engineering')
    pass('empty whitelist still discovers private, group and channel chats automatically without creating bindings')

    await openBot(work.id)
    await until(
      () => browser.evaluate('document.querySelectorAll("[data-telegram-room]").length'),
      (count) => count === 3,
      'three discovered chat rows',
    )
    await browser.select('[data-testid="telegram-kind-filter"]', 'direct')
    await until(
      () => browser.evaluate('document.querySelectorAll("[data-telegram-room]").length'),
      (count) => count === 1,
      'private chat filter',
    )
    await browser.select('[data-testid="telegram-kind-filter"]', '')
    await browser.fill('[data-testid="telegram-chat-search"]', 'Announcements')
    await until(
      () => browser.evaluate('document.querySelectorAll("[data-telegram-room]").length'),
      (count) => count === 1,
      'chat search',
    )
    await browser.fill('[data-testid="telegram-chat-search"]', '')
    await browser.check(chatCheck(77))
    await browser.check(chatCheck(-100123))
    await browser.check(chatCheck(-100456))
    await saveAccess(work.id, (bot) => bot.chat_access.mode === 'whitelist' && bot.chat_access.chat_ids.length === 3)
    const direct = await waitBinding(work.source_session_id, work.id, 77)
    const group = await waitBinding(work.source_session_id, work.id, -100123)
    assert.ok(
      !(await integrations(work.source_session_id, work.id)).some(
        (binding) => String(binding.spec.resource) === '-100456',
      ),
    )
    assert.equal(direct.owner_bot_id, atlas.id)
    assert.equal(group.owner_bot_id, atlas.id)
    assert.equal(direct.spec.channel_policy.identity.account, 'work_bot')
    pass('selecting whitelist chats creates and enables owned bindings; disabled channel kind stays blocked')

    await browser.check('[data-testid="telegram-channel-enabled"]')
    await saveAccess(work.id, (bot) => bot.chat_access.channel_enabled)
    const channel = await waitBinding(work.source_session_id, work.id, -100456)
    for (const [kind, field, chatId, original] of [
      ['private', 'private_enabled', 77, direct],
      ['group', 'group_enabled', -100123, group],
      ['channel', 'channel_enabled', -100456, channel],
    ]) {
      await browser.check('[data-testid="telegram-' + kind + '-enabled"]', false)
      await saveAccess(work.id, (bot) => !bot.chat_access[field])
      assert.equal((await waitBinding(work.source_session_id, work.id, chatId, 'paused')).id, original.id)
      await browser.check('[data-testid="telegram-' + kind + '-enabled"]')
      await saveAccess(work.id, (bot) => bot.chat_access[field])
      assert.equal((await waitBinding(work.source_session_id, work.id, chatId)).id, original.id)
    }
    pass('private, group and channel switches pause and restore their existing routes independently')

    await openBot(home.id)
    await browser.check(chatCheck(77))
    await saveAccess(home.id, (bot) => bot.chat_access.chat_ids.includes('77'))
    const homeDirect = await waitBinding(home.source_session_id, home.id, 77)
    assert.notEqual(homeDirect.id, direct.id)
    assert.equal(homeDirect.owner_bot_id, muse.id)
    assert.equal(homeDirect.spec.channel_policy.identity.account, 'home_bot')
    const homeInventory = await command('bot_inventory', { bot_id: home.id })
    assert.equal(homeInventory.rooms.length, 1)
    assert.equal(homeInventory.rooms[0].last_text, 'home private message')
    assert.equal(
      (await command('bot_inventory', { bot_id: work.id })).rooms.find((room) => room.chat_id === '77').last_text,
      'work private message',
    )
    pass('the same numeric private chat stays isolated between Telegram accounts and AI owners')

    await openBot(work.id)
    await browser.check('[data-testid="telegram-mode-blacklist"]')
    await browser.check(chatCheck(-100123), false)
    await browser.check(chatCheck(-100456), false)
    await saveAccess(
      work.id,
      (bot) =>
        bot.chat_access.mode === 'blacklist' &&
        bot.chat_access.chat_ids.length === 1 &&
        bot.chat_access.chat_ids[0] === '77',
    )
    await waitBinding(work.source_session_id, work.id, 77, 'paused')
    await waitBinding(work.source_session_id, work.id, -100123)
    seed('123:fixture-work', 4, -100999, 'supergroup', 'future room first discovered after blacklist was saved')
    await until(
      () => command('bot_inventory', { bot_id: work.id }),
      (value) => value.rooms.some((room) => room.chat_id === '-100999'),
      'new blacklist-mode chat automatically discovered',
    )
    const future = await waitBinding(work.source_session_id, work.id, -100999)
    assert.equal(future.owner_bot_id, atlas.id)
    await openBot(work.id)
    await browser.check(chatCheck(-100999))
    await saveAccess(work.id, (bot) => bot.chat_access.chat_ids.includes('-100999'))
    await waitBinding(work.source_session_id, work.id, -100999, 'paused')
    await browser.check(chatCheck(-100999), false)
    await saveAccess(work.id, (bot) => !bot.chat_access.chat_ids.includes('-100999'))
    assert.equal((await waitBinding(work.source_session_id, work.id, -100999)).id, future.id)
    pass('blacklist applies to future chats automatically; blocking and unblocking keeps one binding per room')

    await browser.check('[data-testid="telegram-mode-whitelist"]')
    await saveAccess(work.id, (bot) => bot.chat_access.mode === 'whitelist')
    await manualChat('-100888', 'group', 'Manual numeric room')
    await browser.waitVisible(chatCheck(-100888))
    assert.ok(await browser.evaluate('document.querySelector(' + JSON.stringify(chatCheck(-100888)) + ').checked'))
    assert.ok(
      !(await integrations(work.source_session_id, work.id)).some(
        (binding) => String(binding.spec.resource) === '-100888',
      ),
    )
    await saveAccess(work.id, (bot) => bot.chat_access.chat_ids.includes('-100888'))
    await waitBinding(work.source_session_id, work.id, -100888)
    await manualChat('@public_team', 'group', '')
    await browser.waitVisible(chatCheck(-100777))
    await saveAccess(work.id, (bot) => bot.chat_access.chat_ids.includes('-100777'))
    await waitBinding(work.source_session_id, work.id, -100777)
    assert.ok(accounts.get('123:fixture-work').lookups.includes('@public_team'))
    pass('manual numeric IDs and public usernames enter the list, then explicit save authorizes them')

    await browser.select('[data-testid="telegram-ai-bot"]', muse.id)
    const rebound = await saveAccess(work.id, (bot) => bot.ai_bot_id === muse.id)
    assert.equal(rebound.source_session_id, home.source_session_id)
    await until(
      () => integrations(work.source_session_id, work.id),
      (bindings) => bindings.every((binding) => binding.state !== 'enabled'),
      'old AI source paused after reassignment',
    )
    const reboundDirect = await waitBinding(home.source_session_id, work.id, 77)
    assert.equal(reboundDirect.owner_bot_id, muse.id)
    assert.notEqual(reboundDirect.id, homeDirect.id)
    assert.notEqual(reboundDirect.id, direct.id)
    assert.equal((await waitBinding(home.source_session_id, home.id, 77)).id, homeDirect.id)
    pass('changing the AI Bot pauses the old source and creates owned routes without crossing Telegram accounts')
    await browser.select('[data-testid="telegram-ai-bot"]', atlas.id)
    await saveAccess(work.id, (bot) => bot.ai_bot_id === atlas.id)
    assert.equal((await waitBinding(work.source_session_id, work.id, 77)).id, direct.id)
    await waitBinding(home.source_session_id, work.id, 77, 'paused')
    pass('switching back resumes the original policy-paused route without duplicating its binding')

    await openBot(work.id)
    assert.ok(await noOverflow())
    await browser.screenshot(path.join(tmp, 'chats-desktop.png'))
    await browser.evaluate("document.documentElement.classList.add('dark')")
    await browser.screenshot(path.join(tmp, 'chats-dark.png'))
    await browser.send('Emulation.setDeviceMetricsOverride', {
      width: 390,
      height: 844,
      deviceScaleFactor: 1,
      mobile: true,
    })
    assert.ok(await noOverflow(), 'mobile chat policy must fit the viewport')
    await browser.screenshot(path.join(tmp, 'chats-mobile.png'))
    await browser.click('[data-testid="telegram-edit-bot"]')
    await browser.waitVisible('[data-testid="telegram-bot-editor"]')
    assert.ok(await noOverflow(), 'mobile bot editor must fit the viewport')
    await browser.screenshot(path.join(tmp, 'mobile-bot-editor.png'))
    await browser.key('Escape', 'Escape', 27)
    await browser.waitVisible('[data-testid="telegram-bot-editor"]', false)
    await browser.click('[data-testid="telegram-add-chat"]')
    await browser.waitVisible('[data-testid="telegram-manual-chat"]')
    assert.ok(await noOverflow(), 'manual chat dialog must fit the viewport')
    await browser.screenshot(path.join(tmp, 'mobile-manual-chat.png'))
    await browser.key('Escape', 'Escape', 27)
    await browser.waitVisible('[data-testid="telegram-manual-chat"]', false)
    await browser.send('Emulation.setDeviceMetricsOverride', {
      width: 1440,
      height: 1000,
      deviceScaleFactor: 1,
      mobile: false,
    })
    pass('desktop, dark and 390px mobile screens and both dialogs fit without horizontal overflow')

    const savedCatalog = await command('list_bots')
    await stop(server)
    await start()
    const restored = await command('list_bots')
    assert.equal(restored.bots.length, 2)
    for (const old of savedCatalog.bots) {
      const next = restored.bots.find((bot) => bot.id === old.id)
      assert.equal(next.ai_bot_id, old.ai_bot_id)
      assert.equal(next.source_session_id, old.source_session_id)
      assert.deepEqual(next.chat_access, old.chat_access)
      assert.equal((await command('check_connection', { bot_id: next.id })).bot_id, next.id === work.id ? '123' : '456')
    }
    assert.equal((await command('bot_inventory', { bot_id: work.id })).rooms.length, 6)
    assert.equal((await command('bot_inventory', { bot_id: home.id })).rooms.length, 1)
    assert.equal((await waitBinding(work.source_session_id, work.id, 77)).id, direct.id)
    await openBot(work.id)
    assert.equal(await browser.evaluate('document.querySelector("[data-testid=telegram-ai-bot]").value'), atlas.id)
    pass('server restart preserves tokens, AI assignment, policy, discovered rooms and stable routing bindings')

    await browser.click('[data-testid="telegram-toggle-bot"]')
    await until(
      () => catalogBot(work.id),
      (bot) => !bot.enabled,
      'Telegram account disabled',
    )
    await until(
      () => integrations(work.source_session_id, work.id),
      (bindings) => bindings.every((binding) => binding.state !== 'enabled'),
      'disabled account routes paused',
    )
    await until(
      () => browser.evaluate('document.querySelector("[data-testid=telegram-sync]")?.disabled'),
      Boolean,
      'disabled account sync button disabled',
    )
    assert.equal((await waitBinding(home.source_session_id, home.id, 77)).id, homeDirect.id)
    await browser.click('[data-testid="telegram-toggle-bot"]')
    await until(
      () => catalogBot(work.id),
      (bot) => bot.enabled,
      'Telegram account enabled',
    )
    assert.equal((await waitBinding(work.source_session_id, work.id, 77)).id, direct.id)
    pass('disabling one Telegram account pauses its routes and leaves the other account active')

    await openBot(home.id, 'settings')
    await browser.click('[data-testid="telegram-toggle-bot"]')
    await until(
      () => catalogBot(home.id),
      (bot) => !bot.enabled,
      'account disabled before removal',
    )
    await until(
      () => integrations(home.source_session_id, home.id),
      (bindings) => bindings.every((binding) => binding.state !== 'enabled'),
      'removed account routes stopped',
    )
    await browser.click('[data-testid="telegram-remove-bot"]')
    await browser.waitVisible('[role="dialog"]')
    await browser.click('[role="dialog"] button:last-of-type')
    await until(
      () => command('list_bots'),
      (value) => value.bots.length === 1 && value.bots[0].id === work.id,
      'account removed in Web settings',
    )
    assert.equal((await waitBinding(work.source_session_id, work.id, 77)).id, direct.id)
    pass('removing the disabled second Telegram account preserves the first account and its routes')

    assert.deepEqual(errors, [], 'browser JavaScript errors')
    assert.equal(accounts.get('123:fixture-work').sent.length, 0)
    assert.equal(accounts.get('456:fixture-home').sent.length, 0)
    for (const source of [work.source_session_id, home.source_session_id]) {
      const owned = await api('GET', '/api/v1/sessions/' + source)
      assert.equal(owned.owner_surface, 'local')
    }
    const serverLog = await fs.readFile(path.join(tmp, 'server.log'), 'utf8')
    const ordinaryConfig = await fs.readFile(path.join(data, 'agena.json'), 'utf8')
    for (const secret of ['fixture-work', 'fixture-home']) {
      assert.ok(!serverLog.includes(secret), 'server logs must not contain saved tokens')
      assert.ok(!ordinaryConfig.includes(secret), 'ordinary config must not contain saved tokens')
    }
    pass('zero browser exceptions, zero replies to pre-authorization messages and no token leakage')
  } catch (error) {
    failure = error
    console.error(error.stack || error)
    if (browser) {
      await browser.screenshot(path.join(tmp, 'failure.png')).catch(() => {})
      await fs.writeFile(
        path.join(tmp, 'failure.json'),
        JSON.stringify(
          await browser.evaluate('({url:location.href,text:document.body.innerText})').catch(() => ({})),
          null,
          2,
        ),
      )
    }
  } finally {
    browser?.close()
    await stop(chrome)
    await stop(server)
    mock.closeAllConnections()
    await new Promise((resolve) => mock.close(resolve))
    await logfile.close()
    await fs.writeFile(
      path.join(tmp, 'report.json'),
      JSON.stringify(
        { passed: !failure, checks, pageErrors: errors, httpErrors, error: failure?.message || null },
        null,
        2,
      ),
    )
    console.log('Telegram probe artifacts: ' + tmp)
  }
  if (failure) throw failure
}
main().catch((error) => {
  console.error(error.message || error)
  process.exitCode = 1
})
