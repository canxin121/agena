// Real authenticated Agena + Chrome, with two local Telegram Bot API accounts.
// No production data, Telegram account, or external model provider is used.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const os = require('node:os')
const http = require('node:http')
const net = require('node:net')
const crypto = require('node:crypto')
const { spawn } = require('node:child_process')
const { once } = require('node:events')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright')
async function until(read, predicate, title, timeout = 45000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) {
    const result = await read()
    if (predicate(result)) return result
    await new Promise((r) => setTimeout(r, 150))
  }
  throw new Error('Timed out: ' + title)
}
async function freePort() {
  const server = net.createServer()
  await new Promise((r) => server.listen(0, '127.0.0.1', r))
  const port = server.address().port
  await new Promise((r) => server.close(r))
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
async function main() {
  const repo = path.resolve(__dirname, '../../..'),
    tmp = await fs.mkdtemp(path.join(os.tmpdir(), 'agena-telegram-probe-'))
  const data = path.join(tmp, 'data'),
    workspace = path.join(tmp, 'workspace')
  await fs.mkdir(data, { recursive: true })
  await fs.mkdir(workspace, { recursive: true })
  await fs.writeFile(
    path.join(data, 'agena.json'),
    JSON.stringify({ runtime: { providers: { client_versions: { auto_update: false } } } }),
  )
  const accounts = new Map([
    ['123:fixture-work', { id: 123, username: 'work_bot', updates: [], offsets: [], sent: [] }],
    ['456:fixture-home', { id: 456, username: 'home_bot', updates: [], offsets: [], sent: [] }],
  ])
  const mock = http.createServer(async (request, response) => {
    const chunks = []
    for await (const chunk of request) chunks.push(chunk)
    const body = chunks.length ? JSON.parse(Buffer.concat(chunks)) : {}
    const parts = request.url.split('/'),
      token = parts[1]?.slice(3),
      method = parts[2],
      bot = accounts.get(token)
    let result
    if (!bot) {
      response
        .writeHead(401, { 'content-type': 'application/json' })
        .end(JSON.stringify({ ok: false, error_code: 401, description: 'Unauthorized' }))
      return
    }
    if (method === 'getMe')
      result = {
        id: bot.id,
        username: bot.username,
        is_bot: true,
        first_name: bot.username,
        can_join_groups: true,
        can_read_all_group_messages: false,
      }
    else if (method === 'getWebhookInfo') result = { url: '', pending_update_count: bot.updates.length }
    else if (method === 'getUpdates') {
      bot.offsets.push(body.offset ?? null)
      if (body.offset) bot.updates = bot.updates.filter((update) => update.update_id >= body.offset)
      result = bot.updates.slice(0, body.limit || 100)
    } else if (method === 'getChat') {
      const id = Number(body.chat_id)
      result = {
        id,
        type: id > 0 ? 'private' : id === -100456 ? 'channel' : 'supergroup',
        title: id > 0 ? 'Alice' : id === -100456 ? 'Announcements' : 'Team space',
        username: id > 0 ? 'alice' : 'team_space',
        description: 'Local Telegram test chat',
      }
    } else if (method === 'getChatMember')
      result = {
        status: 'administrator',
        user: { id: bot.id, is_bot: true },
        can_post_messages: true,
        can_manage_topics: true,
      }
    else if (method === 'sendMessage') {
      bot.sent.push(body)
      result = { message_id: 999 }
    } else {
      response
        .writeHead(404, { 'content-type': 'application/json' })
        .end(JSON.stringify({ ok: false, error_code: 404, description: 'Unknown method' }))
      return
    }
    response.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({ ok: true, result }))
  })
  await new Promise((r) => mock.listen(0, '127.0.0.1', r))
  const botBase = 'http://127.0.0.1:' + mock.address().port
  const port = await freePort(),
    origin = 'http://127.0.0.1:' + port,
    password = crypto.randomBytes(24).toString('hex')
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('AGENA_')))
  Object.assign(env, { AGENA_DATA_DIR: data, AGENA_SERVER_PASSWORD: password, AGENA_MCP_ENABLED: 'false' })
  const logfile = await fs.open(path.join(tmp, 'server.log'), 'a')
  const checks = [],
    errors = []
  const pass = (name) => {
    checks.push({ name, passed: true })
    console.log('PASS ' + name)
  }
  let server, browser, page, token, failure
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
    assert.ok(response.ok, `${method} ${url}: ${response.status} ${text}`)
    return text ? JSON.parse(text) : null
  }
  async function command(name, input = {}) {
    const response = await api('POST', `/api/v1/plugins/agena.telegram/commands/${name}/invoke`, {
      input,
      session_id: null,
      slash: null,
      raw: '',
    })
    assert.equal(response.result.status, 'succeeded', JSON.stringify(response))
    return response.result.output
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
        if (server.exitCode !== null) throw new Error('Server exited')
        return request('GET', '/auth/session', undefined, false)
          .then((r) => [200, 401].includes(r.status))
          .catch(() => false)
      },
      Boolean,
      'isolated server',
      120000,
    )
    token = (await api('POST', '/auth/session', { password })).token
  }
  async function login() {
    await until(
      async () =>
        (await page.locator('#password').isVisible()) ||
        (await page.locator('[data-testid="telegram-manager"]').isVisible()),
      Boolean,
      'login or authenticated workspace',
    )
    if (await page.locator('#password').isVisible()) {
      await page.locator('#password').fill(password)
      await page.locator('#password').press('Enter')
      await page.locator('[data-testid="telegram-manager"]').waitFor({ timeout: 45000 })
    }
  }

  async function addBot(name, username, botToken) {
    await page.locator('[data-testid="telegram-add-bot"]').click()
    await page.locator('[data-testid="telegram-bot-name"]').fill(name)
    await page.locator('[data-testid="telegram-account"]').fill(username)
    await page.locator('[data-testid="telegram-token"]').fill(botToken)
    await page.locator('[data-testid="telegram-bot-editor"] summary').click()
    await page.locator('[data-testid="telegram-api-base"]').fill(botBase)
    await page.locator('[data-testid="telegram-loopback"]').check()
    await page.locator('[data-testid="telegram-save"]').click()
    await page.locator('[data-testid="telegram-bot-editor"]').waitFor({ state: 'hidden', timeout: 45000 })
    const catalog = await command('list_bots')
    const bot = catalog.bots.find((bot) => bot.connection.account === username)
    assert.ok(bot)
    return bot
  }
  function seed(botToken, id, chat, type, text) {
    const bot = accounts.get(botToken),
      key = type === 'channel' ? 'channel_post' : 'message'
    bot.updates.push({
      update_id: id,
      [key]: {
        message_id: id,
        date: Math.floor(Date.now() / 1000) - 120,
        chat: {
          id: chat,
          type,
          title: type === 'private' ? 'Alice' : type === 'channel' ? 'Announcements' : 'Team space',
          username: type === 'private' ? 'alice' : undefined,
        },
        ...(type === 'channel' ? {} : { from: { id: 77, first_name: 'Alice', is_bot: false } }),
        text,
        ...(type === 'supergroup' ? { message_thread_id: 42, forum_topic_created: { name: 'Engineering' } } : {}),
      },
    })
  }
  async function createBinding(source, owner, kind, chatId, title) {
    await page.goto(origin + `/plugins/agena.telegram/manage?source=${source}&bot=${owner.id}&view=chats`)
    await page.locator('[data-testid="telegram-manager"]').waitFor()
    await until(() => page.locator('[data-testid="telegram-add-chat"]').isEnabled(), Boolean, 'binding creation ready')
    await page.locator('[data-testid="telegram-add-chat"]').click()
    await page.locator('[data-testid="telegram-binding-title"]').fill(title)
    await page.locator('[data-testid="telegram-chat-kind"]').selectOption(kind)
    await page.locator('[data-testid="telegram-chat-id"]').fill(String(chatId))
    await page.locator('[data-testid="telegram-owner-id"]').fill('77')
    await page.locator('[data-testid="telegram-members"]').fill('88')
    await page.locator('[data-testid="telegram-source-consent"]').check()
    await page.locator('[data-testid="telegram-create"]').click()
    await page.locator('[data-testid="telegram-create-form"]').waitFor({ state: 'hidden', timeout: 45000 })
    await page.locator('[data-testid="telegram-binding-detail"]').waitFor()
    const result = await api('GET', `/api/v1/sessions/${source}/integrations`)
    const binding = result.bindings.find((binding) => binding.spec.title === title)
    assert.ok(binding)
    assert.equal(binding.state, 'disabled')
    assert.deepEqual(binding.spec.config, { bot_id: owner.id })
    assert.equal(binding.spec.channel_policy.identity.account, owner.connection.account)
    assert.deepEqual(binding.spec.channel_policy.grant.tools, [])
    return binding
  }
  try {
    await start()
    assert.equal(
      (await request('POST', '/api/v1/plugins/agena.telegram/commands/list_bots/invoke', { input: {} }, false)).status,
      401,
    )
    browser = await chromium.launch({
      headless: true,
      executablePath: process.env.CHROME_BIN || undefined,
      args: ['--no-sandbox', '--disable-dev-shm-usage'],
    })
    const context = await browser.newContext({
      viewport: { width: 1440, height: 1000 },
      locale: 'en-US',
      colorScheme: 'light',
    })
    page = await context.newPage()
    page.on('pageerror', (error) => errors.push(error.message))
    await page.goto(origin + '/plugins/agena.telegram/manage')
    await page.locator('#password').waitFor()
    await login()
    await page.locator('[data-testid="telegram-bot-list"]').waitFor()
    const work = await addBot('Work assistant', 'work_bot', '123:fixture-work')
    await page.locator('[data-testid="telegram-verify"]').click()
    await until(
      () => command('list_bots'),
      (value) => value.bots.find((bot) => bot.id === work.id)?.verification?.bot_id === '123',
      'work bot verified',
    )
    const home = await addBot('Home assistant', 'home_bot', '456:fixture-home')
    await page.locator('[data-testid="telegram-verify"]').click()
    await until(
      () => command('list_bots'),
      (value) => value.bots.find((bot) => bot.id === home.id)?.verification?.bot_id === '456',
      'home bot verified',
    )
    const catalog = await command('list_bots')
    assert.equal(catalog.bots.length, 2)
    const wire =
      JSON.stringify(catalog) + (await page.evaluate(() => document.body.innerText + JSON.stringify(localStorage)))
    assert.ok(
      !wire.includes('fixture-work') && !wire.includes('fixture-home'),
      'write-only tokens must not be returned or persisted in the browser',
    )
    pass('two bots created through the Web UI, credentials stored server-side, independent getMe verification')
    const conflict = await request('POST', '/api/v1/plugins/agena.telegram/commands/save_bot/invoke', {
      input: {
        expected_revision: 0,
        bot: {
          id: work.id,
          name: 'stale overwrite',
          enabled: true,
          credential_kind: 'vault',
          connection: work.connection,
        },
        token: null,
      },
      session_id: null,
      raw: '',
    })
    assert.ok(!conflict.ok || (await conflict.json()).result.status !== 'succeeded')
    assert.equal((await command('list_bots')).bots.find((bot) => bot.id === work.id).name, 'Work assistant')
    pass('stale settings writes are rejected rather than overwriting another bot configuration')
    seed('123:fixture-work', 1, 77, 'private', 'work private message')
    seed('123:fixture-work', 2, -100123, 'supergroup', 'work group message')
    seed('123:fixture-work', 3, -100456, 'channel', 'work channel message')
    seed('456:fixture-home', 1, 77, 'private', 'home private message')
    await page.locator(`[data-telegram-bot="${work.id}"]`).click()
    await page.locator('[data-testid="telegram-sync"]').click()
    await until(
      () => command('bot_inventory', { bot_id: work.id }),
      (value) => value.rooms.length === 3,
      'work rooms discovered',
    )
    await page.locator('[data-telegram-tab="chats"]').click()
    await until(
      () => page.locator('[data-telegram-chat]').count(),
      (value) => value === 3,
      'three room cards',
    )
    await page.locator('[data-testid="telegram-kind-filter"]').selectOption('direct')
    assert.equal(await page.locator('[data-telegram-chat]').count(), 1)
    await page.locator('[data-testid="telegram-kind-filter"]').selectOption('')
    await page.locator('[data-testid="telegram-chat-search"]').fill('Announcements')
    assert.equal(await page.locator('[data-telegram-chat]').count(), 1)
    await page.locator('[data-testid="telegram-chat-search"]').fill('')
    await page.locator('[data-testid="telegram-inspect-id"]').fill('-100123')
    await page.locator('[data-testid="telegram-inspect"]').click()
    await page.locator('[data-testid="telegram-chat-inspector"]').getByText('administrator', { exact: true }).waitFor()
    pass('private/group/channel discovery, type and text filters, topic metadata and bot membership inspection')
    await page.locator(`[data-telegram-bot="${home.id}"]`).click()
    await page.locator('[data-testid="telegram-sync"]').click()
    await until(
      () => command('bot_inventory', { bot_id: home.id }),
      (value) => value.rooms.length === 1,
      'home private chat',
    )
    await page.locator('[data-telegram-tab="activity"]').click()
    await page.getByText('home private message', { exact: true }).waitFor()
    assert.equal(await page.getByText('work private message', { exact: true }).count(), 0)
    await page.locator(`[data-telegram-bot="${work.id}"]`).click()
    await page.getByText('work private message', { exact: true }).waitFor()
    assert.equal(await page.getByText('home private message', { exact: true }).count(), 0)
    pass('same numeric private-chat ID remains isolated between bot inventories and message views')
    const resolved = await api('POST', '/api/v1/workspaces/resolve', { path: workspace, create_if_missing: true })
    const session = await api('POST', '/api/v1/sessions', {
      workspace_id: resolved.id,
      title: 'Telegram operators',
      auto_title: false,
    })
    const direct = await createBinding(session.id, work, 'direct', 77, 'Work private authorization')
    await createBinding(session.id, work, 'group', -100123, 'Work group authorization')
    await createBinding(session.id, work, 'broadcast', -100456, 'Work channel authorization')
    await createBinding(session.id, home, 'direct', 77, 'Home private authorization')
    const grants = await api('GET', `/api/v1/sessions/${session.id}/integrations`)
    assert.equal(grants.bindings.filter((binding) => binding.spec.provider === 'agena.telegram').length, 4)
    pass('two bots and four private/group/channel grants created with explicit consent and empty tool permissions')
    await page.goto(origin + `/plugins/agena.telegram/manage?source=${session.id}&bot=${work.id}&view=chats`)
    await page.locator('[data-telegram-chat="77"]').click()
    await page.locator('[data-testid="telegram-binding-detail"]').waitFor()
    assert.ok(
      (await page.locator('[data-testid="telegram-binding-detail"]').innerText()).includes(
        'Work private authorization',
      ),
    )
    assert.ok(
      !(await page.locator('[data-testid="telegram-binding-detail"]').innerText()).includes(
        'Home private authorization',
      ),
    )
    await page.locator('[data-testid="telegram-enable"]').click()
    await page.getByRole('button', { name: 'Confirm', exact: true }).last().click()
    await until(
      () => api('GET', `/api/v1/sessions/${session.id}/integrations`),
      (value) => value.bindings.find((binding) => binding.id === direct.id)?.state === 'enabled',
      'binding enabled',
    )
    await page.getByRole('button', { name: /Members and access/ }).click()
    await page.locator('[data-testid="channel-access-panel"]').waitFor()
    await page.reload()
    await page.locator('[data-telegram-chat="77"]').waitFor()
    await page.locator('[data-telegram-chat="77"]').click()
    await page.locator('[data-testid="telegram-binding-detail"]').waitFor()
    pass(
      'scoped authorization controls, access-policy inspection and deep-link refresh remain bound to the selected bot',
    )
    await page.locator('[data-telegram-tab="overview"]').click()
    await page.screenshot({ path: path.join(tmp, 'overview-desktop.png'), fullPage: true })
    await page.locator('[data-telegram-tab="chats"]').click()
    await page.screenshot({ path: path.join(tmp, 'chats-desktop.png'), fullPage: true })
    await page.evaluate(() => document.documentElement.classList.add('dark'))
    await page.screenshot({ path: path.join(tmp, 'chats-dark.png'), fullPage: true })
    await page.setViewportSize({ width: 390, height: 844 })
    await page.screenshot({ path: path.join(tmp, 'mobile.png'), fullPage: true })
    assert.ok(
      await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1),
      'mobile layout must not overflow',
    )
    await page.locator(`[data-telegram-bot="${home.id}"]`).click()
    await page.locator('[data-telegram-tab="activity"]').click()
    await page.getByText('home private message', { exact: true }).waitFor()
    await page.locator('[data-testid="telegram-edit-bot"]').click()
    await page.locator('[data-testid="telegram-bot-name"]').waitFor()
    await page.screenshot({ path: path.join(tmp, 'mobile-bot-editor.png'), fullPage: true })
    assert.ok(
      await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1),
      'mobile bot editor must fit',
    )
    await page.keyboard.press('Escape')
    await page.locator('[data-testid="telegram-bot-editor"]').waitFor({ state: 'hidden' })
    await page.setViewportSize({ width: 1440, height: 1000 })
    pass(
      'desktop, dark-mode and mobile layouts, bot switching, editable dialog and keyboard dismissal without overflow',
    )
    await stop(server)
    await start()
    const restored = await command('list_bots')
    assert.equal(restored.bots.length, 2)
    assert.equal((await command('check_connection', { bot_id: work.id })).bot_id, '123')
    assert.equal((await command('check_connection', { bot_id: home.id })).bot_id, '456')
    assert.equal((await command('bot_inventory', { bot_id: work.id })).rooms.length, 3)
    assert.equal((await command('bot_inventory', { bot_id: home.id })).rooms.length, 1)
    pass(
      'server restart preserves both credential-store tokens, discovered rooms and independently verified identities',
    )
    await page.goto(origin + `/plugins/agena.telegram/manage?source=${session.id}&bot=${home.id}&view=settings`)
    await login()
    await page.locator('[data-testid="telegram-toggle-bot"]').waitFor()
    await page.locator('[data-testid="telegram-toggle-bot"]').click()
    await page.getByRole('button', { name: 'Confirm', exact: true }).last().click()
    await until(
      () => command('list_bots'),
      (value) => value.bots.find((bot) => bot.id === home.id)?.enabled === false,
      'home bot disabled',
    )
    const disabled = await request('POST', '/api/v1/plugins/agena.telegram/commands/sync_chats/invoke', {
      input: { bot_id: home.id },
      session_id: null,
      raw: '',
    })
    assert.ok(!disabled.ok || (await disabled.json()).result.status !== 'succeeded')
    await page.locator('[data-testid="telegram-remove-bot"]').click()
    await page.getByRole('button', { name: 'Remove bot', exact: true }).last().click()
    await until(
      () => command('list_bots'),
      (value) => value.bots.length === 1 && value.bots[0].id === work.id,
      'home removed',
    )
    const stale = await request('POST', '/api/v1/plugins/agena.telegram/commands/check_connection/invoke', {
      input: { bot_id: home.id },
      session_id: null,
      raw: '',
    })
    assert.ok(!stale.ok || (await stale.json()).result.status !== 'succeeded')
    pass('disabled and removed bot identities fail closed while unrelated bots remain available')
    assert.deepEqual(errors, [], 'browser JavaScript errors')
    assert.equal(
      accounts.get('123:fixture-work').sent.length,
      0,
      'discovery and old messages never trigger unsolicited replies',
    )
    const serverLog = await fs.readFile(path.join(tmp, 'server.log'), 'utf8')
    const ordinaryConfig = await fs.readFile(path.join(data, 'agena.json'), 'utf8')
    assert.ok(
      !serverLog.includes('fixture-work') && !serverLog.includes('fixture-home'),
      'server logs must not contain bot credentials',
    )
    assert.ok(
      !ordinaryConfig.includes('fixture-work') && !ordinaryConfig.includes('fixture-home'),
      'ordinary config must not contain bot credentials',
    )
    pass('zero browser errors and discovery never grants authorization or sends unsolicited messages')
  } catch (error) {
    failure = error
    console.error(error.stack || error)
    if (page && !page.isClosed()) {
      await page.screenshot({ path: path.join(tmp, 'failure.png'), fullPage: true }).catch(() => {})
      await fs.writeFile(
        path.join(tmp, 'failure.json'),
        JSON.stringify(
          await page.evaluate(() => ({ url: location.href, text: document.body.innerText })).catch(() => ({})),
          null,
          2,
        ),
      )
    }
  } finally {
    await browser?.close().catch(() => {})
    await stop(server)
    mock.closeAllConnections()
    await new Promise((r) => mock.close(r))
    await logfile.close()
    await fs.writeFile(
      path.join(tmp, 'report.json'),
      JSON.stringify({ passed: !failure, checks, pageErrors: errors, error: failure?.message || null }, null, 2),
    )
    console.log('Telegram probe artifacts: ' + tmp)
  }
  if (failure) throw failure
}
main().catch((error) => {
  console.error(error.message || error)
  process.exitCode = 1
})
