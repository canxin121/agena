// Isolated external MCP client REST + browser regression probe. Requires a built
// Agena binary, a production Web build, Playwright and Chrome/Chromium. All
// connections and credentials are synthetic; all processes and files belong to
// a temporary server instance. This never restarts or contacts the live service.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const os = require('node:os')
const net = require('node:net')
const { spawn } = require('node:child_process')
const { once } = require('node:events')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright')

async function until(read, predicate, label, attempts = 600) {
  let last
  for (let attempt = 0; attempt < attempts; attempt++) {
    last = await read()
    if (predicate(last)) return last
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error('Timed out: ' + label + '\nLast result: ' + JSON.stringify(last).slice(0, 1500))
}

async function stop(child) {
  if (!child?.pid || child.exitCode !== null || child.signalCode !== null) return
  const stopped = once(child, 'exit').catch(() => {})
  child.kill('SIGTERM')
  await Promise.race([stopped, new Promise((resolve) => setTimeout(resolve, 5000))])
  if (child.exitCode === null && child.signalCode === null) {
    child.kill('SIGKILL')
    await stopped
  }
}

async function main() {
  const repo = path.resolve(__dirname, '../../..')
  const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'agena-mcp-probe-'))
  const workspace = path.join(temporary, 'server-workspace')
  const other = path.join(temporary, 'chat-directory')
  const data = path.join(temporary, 'data')
  const fixture = path.join(repo, 'crates/agena-mcp-client/tests/fixtures/server.py')
  const python = process.env.PYTHON_BIN || 'python3'
  const children = []
  const logs = []
  let browser
  let page
  let server
  const pageErrors = []
  const checks = []
  await Promise.all([workspace, other, data].map((folder) => fs.mkdir(folder, { recursive: true })))

  async function startHttpFixture(name, oauth = false) {
    const portFile = path.join(temporary, name + '.url')
    const log = await fs.open(path.join(temporary, name + '.log'), 'w')
    logs.push(log)
    const child = spawn(python, [fixture, '--transport', 'http', '--protocol', 'modern', '--capabilities', 'mixed', '--port-file', portFile, ...(oauth ? ['--oauth'] : [])], {
      cwd: temporary, stdio: ['ignore', log.fd, log.fd],
    })
    children.push(child)
    let failure
    child.on('error', (error) => { failure = error })
    return until(async () => {
      if (failure) throw failure
      if (child.exitCode !== null) throw new Error('Fixture exited: ' + name)
      return fs.readFile(portFile, 'utf8').catch(() => '')
    }, Boolean, name + ' readiness')
  }

  try {
    const httpOrigin = await startHttpFixture('http-fixture')
    const oauthOrigin = await startHttpFixture('oauth-fixture', true)
    const listener = net.createServer()
    await new Promise((resolve) => listener.listen(0, '127.0.0.1', resolve))
    const port = listener.address().port
    await new Promise((resolve) => listener.close(resolve))
    const origin = 'http://127.0.0.1:' + port
    const serverLog = await fs.open(path.join(temporary, 'server.log'), 'w')
    logs.push(serverLog)
    server = spawn(process.env.AGENA_BIN || path.join(repo, 'target/debug/agena'), [
      '--data-dir', data, 'server', '--host', '127.0.0.1', '--port', String(port),
      '--workspace', workspace, '--ui-dir', path.join(repo, 'packages/agena-web/dist'),
      '--mcp-enabled', 'false',
    ], {
      cwd: workspace, stdio: ['ignore', serverLog.fd, serverLog.fd],
      env: { ...process.env, AGENA_SERVER_PASSWORD: '', AGENA_SERVER_UI_PASSWORD: '', AGENA_DATA_DIR: data, AGENA_MCP_ENABLED: 'false' },
    })
    children.push(server)
    let serverFailure
    server.on('error', (error) => { serverFailure = error })
    await until(async () => {
      if (serverFailure) throw serverFailure
      if (server.exitCode !== null) throw new Error('Probe server exited: ' + await fs.readFile(path.join(temporary, 'server.log'), 'utf8'))
      return fetch(origin + '/auth/session').then((response) => response.ok).catch(() => false)
    }, Boolean, 'isolated server readiness', 1200)

    const base = '/mcp-connections'
    const route = (id, suffix = '') => base + '/' + encodeURIComponent(id) + suffix
    async function api(method, url, body, status = 200) {
      const response = await fetch(origin + '/api/v1' + url, {
        method, headers: { 'content-type': 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body),
      })
      const text = await response.text()
      assert.equal(response.status, status, method + ' ' + url + ': ' + text)
      return text ? JSON.parse(text) : null
    }
    async function catalog() { return api('GET', base) }
    async function detail(id) { return api('GET', route(id)) }
    async function byName(name, scope = 'workspace') {
      return (await catalog()).connections.find((item) => item.name === name && item.scope === scope)
    }
    async function state(id, expected) {
      return until(() => detail(id), (item) => item.state === expected, expected + ' state for ' + id)
    }
    async function create(name, config, scope = 'workspace') {
      const result = await api('POST', base, { name, scope, config })
      assert.equal(result.saved, true, 'connection saved: ' + name)
      assert.ok(result.connection?.id)
      return result.connection
    }
    const stdioConfig = (enabled = true) => ({
      transport: 'stdio', enabled,
      process: { command: python, args: [fixture, '--protocol', 'modern', '--capabilities', 'mixed'], env: {} },
      tools: { include: [], exclude: [] }, timeouts: { connect_ms: 20000, request_ms: 60000 },
    })
    const httpConfig = (url, auth = null, enabled = true) => ({
      transport: 'http', enabled, endpoint: { url, headers: {} }, auth,
      tools: { include: [], exclude: [] }, timeouts: { connect_ms: 20000, request_ms: 60000 },
    })
    const initial = await catalog()
    assert.equal(await fs.realpath(initial.workspace_root), await fs.realpath(workspace))
    for (const file of [initial.global_path, initial.workspace_path]) {
      assert.ok(path.resolve(file).startsWith(path.resolve(temporary) + path.sep), 'probe configuration must stay in its temporary directory: ' + file)
    }
    if (!initial.bridge_enabled) await api('PUT', base + '/bridge/enabled', { enabled: true, scope: 'workspace' })
    await api('POST', '/workspaces/resolve', { path: other, create_if_missing: true })

    const global = await create('shared-service', stdioConfig(false), 'global')
    const globalOnly = await create('global-helper', stdioConfig(false), 'global')
    const scoped = await create('shared-service', stdioConfig())
    await state(scoped.id, 'connected')
    const merged = await catalog()
    assert.equal(merged.connections.find((item) => item.id === global.id).effective, false)
    assert.equal(merged.connections.find((item) => item.id === globalOnly.id).effective, true)
    assert.equal(merged.connections.find((item) => item.id === scoped.id).effective, true)
    checks.push('global/workspace per-name precedence and actual server workspace')

    const privateConfig = httpConfig(httpOrigin + '/mcp?access_key=fixture-url-secret')
    privateConfig.endpoint.headers.Authorization = 'Bearer fixture-header-secret'
    const privateConnection = await create('private-http', privateConfig)
    let privateDetail = await state(privateConnection.id, 'connected')
    assert.equal(privateDetail.config.endpoint.url, '[REDACTED]')
    assert.ok(privateDetail.secret_paths.includes('/endpoint/url'))
    assert.ok(privateDetail.secret_paths.includes('/endpoint/headers/Authorization'))
    assert.ok(!JSON.stringify(privateDetail).includes('fixture-url-secret'))
    assert.ok(!JSON.stringify(privateDetail).includes('fixture-header-secret'))
    const savedRevision = privateDetail.revision
    privateDetail.config.timeouts.request_ms = 42000
    await api('PUT', route(privateDetail.id), { config: privateDetail.config, preserve_secrets: privateDetail.secret_paths, expected_revision: savedRevision })
    await api('PUT', route(privateDetail.id), { config: privateDetail.config, preserve_secrets: privateDetail.secret_paths, expected_revision: savedRevision }, 409)
    await state(privateDetail.id, 'connected')
    assert.equal((await api('POST', route(privateDetail.id, '/test'), {})).ok, true)
    checks.push('secret URL/header redaction, preservation and revision conflicts')

    browser = await chromium.launch({ executablePath: process.env.CHROME_BIN || undefined, headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] })
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'en-US' })
    context.setDefaultTimeout(20000)
    await context.addInitScript((directory) => {
      localStorage.setItem('studio.ui.locale', 'en-US')
      localStorage.setItem('studio.directory.current-directory-by-window', JSON.stringify({ 'window-default': directory }))
    }, other)
    page = await context.newPage()
    page.on('pageerror', (error) => pageErrors.push(error.message))
    const settingsUrl = (id = '') => origin + '/settings/mcp-connections?view=connections&directory=' + encodeURIComponent(other) + (id ? '&connection=' + encodeURIComponent(id) : '')
    const button = (name) => page.getByRole('button', { name, exact: true })
    const tabs = () => page.getByRole('navigation', { name: 'Connection details', exact: true })
    const tab = (name) => tabs().getByRole('button', { name, exact: true }).click()
    async function openConnection(connection) {
      await page.goto(settingsUrl(connection.id))
      await page.getByRole('heading', { name: connection.name, exact: true }).waitFor()
      await page.getByText('All changes are saved.', { exact: true }).waitFor()
    }
    async function refreshConnected(connection) {
      await state(connection.id, 'connected')
      await button('Refresh connections').click()
      await page.locator('header').filter({ has: page.getByRole('heading', { name: connection.name, exact: true }) }).getByText('Connected', { exact: true }).waitFor()
    }
    async function discardDraft() {
      await button('Discard changes').click()
      const dialog = page.getByRole('dialog')
      await dialog.getByRole('button', { name: 'Discard changes', exact: true }).click()
      await dialog.waitFor({ state: 'hidden' })
      await page.getByText('All changes are saved.', { exact: true }).waitFor()
    }
    async function chooseOption(title, value) {
      await page.getByRole('button', { name: new RegExp('^' + title + ':') }).click()
      await page.locator('[data-option-menu-item]:visible').filter({ hasText: value }).click()
    }
    async function noHorizontalOverflow(label) {
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 2), label + ' should not overflow horizontally')
    }
    async function screenshot(name) { await page.screenshot({ path: path.join(temporary, name), fullPage: true }) }

    await page.goto(settingsUrl())
    await button('Add connection').click()
    let dialog = page.getByRole('dialog')
    await dialog.getByLabel('Connection name', { exact: true }).fill('browser-stdio')
    await dialog.getByLabel('Command', { exact: true }).fill(python)
    for (const [index, argument] of stdioConfig().process.args.entries()) {
      await dialog.getByRole('button', { name: 'Add argument', exact: true }).click()
      await dialog.getByLabel('Argument ' + (index + 1), { exact: true }).fill(argument)
    }
    assert.ok((await dialog.innerText()).includes(workspace), 'create form uses the running server workspace')
    await dialog.getByRole('button', { name: 'Add connection', exact: true }).click()
    await dialog.waitFor({ state: 'hidden' })
    const created = await until(() => byName('browser-stdio'), Boolean, 'browser-created stdio connection')
    await refreshConnected(created)
    await api('POST', base, { name: created.name, scope: created.scope, config: stdioConfig() }, 409)
    assert.ok((await catalog()).workspace_path.startsWith(workspace + path.sep))

    await page.getByText('Timeouts and tool filters', { exact: true }).click()
    await page.getByLabel('Request timeout (ms)', { exact: true }).fill('45000')
    await page.getByLabel('Request timeout (ms)', { exact: true }).press('Control+s')
    await until(() => detail(created.id), (item) => item.config.timeouts.request_ms === 45000, 'keyboard save')
    await page.getByText('All changes are saved.', { exact: true }).waitFor()

    await button('Edit JSON').click()
    await page.getByLabel('Connection JSON', { exact: true }).fill('{')
    assert.equal(await button('Save changes').isDisabled(), true)
    await button('Refresh connections').click()
    assert.equal(await page.getByLabel('Connection JSON', { exact: true }).inputValue(), '{')
    await discardDraft()
    await page.getByLabel('Command', { exact: true }).waitFor()
    await page.getByLabel('Working directory', { exact: true }).fill(other)
    const concurrent = await detail(created.id)
    concurrent.config.timeouts.request_ms = 46000
    await api('PUT', route(created.id), { config: concurrent.config, preserve_secrets: concurrent.secret_paths, expected_revision: concurrent.revision })
    await button('Refresh connections').click()
    await page.getByText('This connection changed elsewhere. Reload the latest configuration before replacing it.', { exact: true }).waitFor()
    assert.equal(await page.getByLabel('Working directory', { exact: true }).inputValue(), other)
    assert.equal(await button('Save changes').isDisabled(), true)
    await discardDraft()
    assert.equal(await page.getByLabel('Working directory', { exact: true }).inputValue(), '')
    checks.push('browser structured create, keyboard save, invalid-draft refresh and conflict recovery')

    await button('Disable').click()
    await state(created.id, 'disabled')
    await button('Enable').waitFor()
    await button('Test connection').click()
    await page.getByText('Connection test passed', { exact: true }).waitFor()
    await button('Enable').click()
    await refreshConnected(created)
    const reconnected = page.waitForResponse((response) => response.url().endsWith(route(created.id, '/reconnect')) && response.request().method() === 'POST')
    await button('Reconnect').click()
    assert.equal((await reconnected).status(), 200)
    await refreshConnected(created)
    checks.push('browser disable, isolated disabled-connection test, enable and reconnect')

    await tab('Tools')
    await button('ping').waitFor()
    assert.ok((await page.locator('pre').filter({ hasText: 'inputSchema' }).innerText()).includes('properties'))
    await screenshot('mcp-tools-desktop.png')
    await tab('Resources')
    await button('Notes').waitFor()
    assert.equal(await page.getByLabel('Resource URI', { exact: true }).inputValue(), 'fixture://notes')
    await button('Read resource').click()
    await page.locator('pre').filter({ hasText: 'Fixture notes' }).waitFor()
    await chooseOption('Resource kind', 'Resource templates')
    await until(() => page.getByLabel('Resource URI', { exact: true }).inputValue(), (uri) => uri.includes('{id}'), 'resource template')
    assert.equal(await button('Read resource').isDisabled(), true)
    await page.getByLabel('Resource URI', { exact: true }).fill('fixture://notes/42')
    await button('Read resource').click()
    await page.locator('pre').filter({ hasText: 'Fixture notes' }).waitFor()
    await tab('Prompts')
    await button('review').waitFor()
    await button('Preview prompt').click()
    await page.locator('pre').filter({ hasText: 'Review fixture notes' }).waitFor()
    await screenshot('mcp-capabilities-desktop.png')
    checks.push('real MCP tools, resources, resource templates and prompt preview')

    await openConnection(privateConnection)
    assert.equal(await page.getByLabel('MCP endpoint URL', { exact: true }).isDisabled(), true)
    assert.equal(await page.getByLabel('MCP endpoint URL', { exact: true }).inputValue(), '')
    await page.getByText('Timeouts and tool filters', { exact: true }).click()
    await page.getByLabel('Request timeout (ms)', { exact: true }).fill('43000')
    await button('Save changes').click()
    await until(() => detail(privateConnection.id), (item) => item.config.timeouts.request_ms === 43000, 'edit with hidden endpoint')
    await page.getByText('All changes are saved.', { exact: true }).waitFor()
    const configPath = (await catalog()).workspace_path
    assert.ok((await fs.readFile(configPath, 'utf8')).includes('fixture-url-secret'))
    await button('Replace endpoint URL').click()
    await page.getByLabel('MCP endpoint URL', { exact: true }).fill(httpOrigin + '/mcp?access_key=replaced-fixture-url')
    await button('Save changes').click()
    await page.getByText('All changes are saved.', { exact: true }).waitFor()
    assert.ok((await fs.readFile(configPath, 'utf8')).includes('replaced-fixture-url'))
    assert.ok(!(await fs.readFile(configPath, 'utf8')).includes('fixture-url-secret'))
    checks.push('browser preserves and replaces hidden endpoint URLs')

    const registration = await create('oauth-registration', httpConfig(httpOrigin + '/mcp', { kind: 'o_auth', scopes: ['read'], client_id: 'registered-client', client_secret: 'fixture-client-secret' }, false))
    await openConnection(registration)
    assert.equal(await page.getByLabel('Client secret (optional)', { exact: true }).isDisabled(), true)
    await page.getByLabel('Client ID', { exact: true }).fill('updated-client')
    await button('Save changes').click()
    await page.getByText('All changes are saved.', { exact: true }).waitFor()
    assert.ok((await detail(registration.id)).secret_paths.includes('/auth/client_secret'))
    await button('Remove client secret').click()
    await page.getByLabel('Client ID', { exact: true }).fill('')
    await page.getByLabel('Client metadata URL', { exact: true }).fill('https://example.invalid/client.json?key=fixture-metadata-secret')
    await button('Save changes').click()
    await page.getByText('All changes are saved.', { exact: true }).waitFor()
    assert.equal((await detail(registration.id)).config.auth.client_secret ?? null, null)
    assert.equal(await page.getByLabel('Client metadata URL', { exact: true }).isDisabled(), true)
    await button('Replace client metadata URL').click()
    await page.getByLabel('Client metadata URL', { exact: true }).fill('https://example.invalid/client.json')
    await button('Save changes').click()
    await page.getByText('All changes are saved.', { exact: true }).waitFor()
    assert.equal((await detail(registration.id)).config.auth.client_metadata_url, 'https://example.invalid/client.json')
    checks.push('optional OAuth registration secrets and protected metadata URL editing')

    const oauth = await create('oauth-service', httpConfig(oauthOrigin + '/mcp', { kind: 'o_auth', scopes: ['read'] }))
    await until(() => detail(oauth.id), (item) => item.state !== 'pending_reload', 'OAuth configuration activated')
    await openConnection(oauth)
    await button('Use manual authorization').click()
    const authorizationLink = page.getByRole('link', { name: 'Open authorization link', exact: true })
    await authorizationLink.waitFor()
    const pending = await page.evaluate(() => JSON.parse(sessionStorage.getItem('agena.mcp.oauth.pending')))
    assert.deepEqual(Object.keys(pending).sort(), ['connection_id', 'expires_at', 'flow_id', 'state'])
    const authorizeResponse = await fetch(await authorizationLink.getAttribute('href'), { redirect: 'manual' })
    assert.equal(authorizeResponse.status, 302)
    const callback = authorizeResponse.headers.get('location')
    assert.ok(callback?.startsWith(origin + '/settings/mcp-connections?'))
    const wrongCallback = new URL(callback)
    wrongCallback.searchParams.set('state', 'wrong-state')
    await page.getByLabel('Callback URL', { exact: true }).fill(wrongCallback.href)
    await button('Complete sign-in').click()
    await page.getByText('The callback URL does not match this sign-in request.', { exact: true }).waitFor()
    await page.getByLabel('Callback URL', { exact: true }).fill(callback)
    await button('Complete sign-in').click()
    await refreshConnected(oauth)
    assert.equal(await page.evaluate(() => sessionStorage.getItem('agena.mcp.oauth.pending')), null)
    await button('Sign out').click()
    dialog = page.getByRole('dialog')
    await dialog.getByRole('button', { name: 'Sign out', exact: true }).click()
    await dialog.waitFor({ state: 'hidden' })
    await until(() => detail(oauth.id), (item) => item.state !== 'connected', 'OAuth logout')
    const completion = page.waitForResponse((response) => response.url().endsWith(route(oauth.id, '/auth/complete')) && response.request().method() === 'POST')
    await button('Sign in').click()
    assert.equal((await completion).status(), 200)
    await state(oauth.id, 'connected')
    await until(() => page.url(), (url) => new URL(url).origin === origin && !new URL(url).searchParams.has('code') && !new URL(url).searchParams.has('state'), 'OAuth callback URL cleanup')
    await page.getByRole('heading', { name: oauth.name, exact: true }).waitFor()
    await page.getByText('All changes are saved.', { exact: true }).waitFor()
    assert.equal(await page.evaluate(() => sessionStorage.getItem('agena.mcp.oauth.pending')), null)
    checks.push('real OAuth PKCE manual callback, state validation, logout and automatic redirect continuation')
    await screenshot('mcp-auth-desktop.png')

    const beforeImport = await detail(created.id)
    await button('Import connections').click()
    dialog = page.getByRole('dialog')
    await dialog.getByLabel('MCP configuration JSON', { exact: true }).fill('null')
    await dialog.getByRole('button', { name: 'Preview import', exact: true }).click()
    await dialog.getByText('Enter a valid JSON object.', { exact: true }).waitFor()
    const importDocument = {
      mcpServers: {
        'imported-service': { command: python, args: stdioConfig().process.args, env: {} },
        'browser-stdio': { command: python, args: ['must-not-replace-existing-configuration'] },
        'invalid-service': { url: 'file:///invalid-mcp-endpoint' },
      },
    }
    await dialog.locator('input[type="file"]').setInputFiles({ name: 'mcp-connections.json', mimeType: 'application/json', buffer: Buffer.from(JSON.stringify(importDocument)) })
    await dialog.getByRole('button', { name: 'Preview import', exact: true }).click()
    await dialog.getByText('Choose connections to import', { exact: true }).waitFor()
    await dialog.getByText('Already configured', { exact: true }).waitFor()
    await dialog.getByText('Invalid configuration', { exact: true }).waitFor()
    await screenshot('mcp-import-desktop.png')
    await dialog.getByRole('button', { name: 'Import selected connections', exact: true }).click()
    await dialog.waitFor({ state: 'hidden' })
    const imported = await until(() => byName('imported-service'), Boolean, 'browser configuration-file import')
    assert.deepEqual((await detail(created.id)).config, beforeImport.config)
    assert.equal(await byName('invalid-service'), undefined)
    await refreshConnected(imported)
    checks.push('browser JSON file preview/import, invalid candidates and conflict preservation')

    const enabledBefore = Object.fromEntries((await catalog()).connections.map((item) => [item.id, item.enabled]))
    await page.getByText('Server workspace and configuration', { exact: true }).click()
    await button('Disable MCP bridge').click()
    await page.getByText('The MCP bridge is disabled', { exact: true }).waitFor()
    assert.equal((await catalog()).bridge_enabled, false)
    assert.deepEqual(Object.fromEntries((await catalog()).connections.map((item) => [item.id, item.enabled])), enabledBefore)
    await button('Enable MCP bridge').click()
    await refreshConnected(imported)
    await page.getByText('The MCP bridge is disabled', { exact: true }).waitFor({ state: 'hidden' })
    checks.push('browser bridge disable/recovery preserves per-connection enable flags')
    await screenshot('mcp-connections-desktop.png')

    await page.setViewportSize({ width: 390, height: 844 })
    await page.goto(settingsUrl())
    await page.getByText('browser-stdio', { exact: true }).first().waitFor()
    await noHorizontalOverflow('mobile connection list')
    await screenshot('mcp-connections-mobile.png')
    await page.getByRole('button').filter({ has: page.getByText('browser-stdio', { exact: true }) }).click()
    await page.getByRole('heading', { name: 'browser-stdio', exact: true }).waitFor()
    await tab('Tools')
    await button('ping').waitFor()
    await noHorizontalOverflow('mobile connection detail')
    await screenshot('mcp-detail-mobile.png')
    checks.push('desktop/mobile loaded views and horizontal overflow checks')

    await page.setViewportSize({ width: 1440, height: 1000 })
    await openConnection(imported)
    await button('Remove connection').click()
    dialog = page.getByRole('dialog')
    await dialog.getByRole('button', { name: 'Remove connection', exact: true }).click()
    await dialog.waitFor({ state: 'hidden' })
    await until(() => byName(imported.name), (item) => !item, 'browser removal')
    await api('GET', route(imported.id), undefined, 404)
    checks.push('browser remove and missing-connection response')
    assert.deepEqual(pageErrors, [])
    console.log(JSON.stringify({ ok: true, temporary, browser: await browser.version(), checks }, null, 2))
  } catch (error) {
    if (page) {
      await page.screenshot({ path: path.join(temporary, 'failure.png'), fullPage: true }).catch(() => {})
      console.error((await page.locator('body').innerText().catch(() => '')).slice(0, 7000))
    }
    console.error('Probe artifacts: ' + temporary)
    throw error
  } finally {
    await browser?.close()
    // Stop Agena first so its stdio MCP children are reaped before HTTP fixtures.
    for (const child of [...children].reverse()) await stop(child)
    await Promise.all(logs.map((log) => log.close()))
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
