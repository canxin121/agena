// Real installed-plugin, authenticated HTTP, and browser regression probe.
// Build agena, agena-plugin-dev, agena-web-dashboard-plugin-stdio and the Web UI.
// Every mutation uses a fresh data root, registry, and server owned by this probe.
const assert = require('node:assert/strict')
const crypto = require('node:crypto')
const fs = require('node:fs/promises')
const http = require('node:http')
const net = require('node:net')
const os = require('node:os')
const path = require('node:path')
const { spawn, spawnSync } = require('node:child_process')
const { once } = require('node:events')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright')

async function until(read, accept, label, timeout = 30000) {
  const deadline = Date.now() + timeout
  let value
  while (Date.now() < deadline) {
    value = await read()
    if (accept(value)) return value
    await new Promise((resolve) => setTimeout(resolve, 150))
  }
  throw new Error('Timed out: ' + label)
}

async function freePort() {
  const listener = net.createServer()
  await new Promise((resolve, reject) => {
    listener.once('error', reject)
    listener.listen(0, '127.0.0.1', resolve)
  })
  const port = listener.address().port
  await new Promise((resolve) => listener.close(resolve))
  return port
}

async function terminate(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  const done = once(child, 'exit').catch(() => {})
  child.kill('SIGTERM')
  let timer
  await Promise.race([
    done,
    new Promise((resolve) => {
      timer = setTimeout(() => {
        child.kill('SIGKILL')
        resolve()
      }, 8000)
    }),
  ])
  clearTimeout(timer)
  await done
}

async function main() {
  const repo = path.resolve(__dirname, '../../..')
  const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'agena-plugin-web-probe-'))
  const data = path.join(temporary, 'data')
  const workspace = path.join(temporary, 'workspace')
  const release = path.join(temporary, 'release')
  await Promise.all([data, workspace, release].map((dir) => fs.mkdir(dir, { recursive: true })))
  await fs.writeFile(
    path.join(data, 'agena.json'),
    JSON.stringify({ runtime: { providers: { client_versions: { auto_update: false } } } }) + '\n',
  )
  const extension = process.platform === 'win32' ? '.exe' : ''
  const binary = process.env.AGENA_BIN || path.join(repo, 'target/debug/agena' + extension)
  const pluginBinary =
    process.env.AGENA_WEB_PLUGIN_BIN || path.join(repo, 'target/debug/agena-web-dashboard-plugin-stdio' + extension)
  const packager = process.env.AGENA_PLUGIN_DEV_BIN || path.join(repo, 'target/debug/agena-plugin' + extension)
  const pluginId = 'demo.web_dashboard'
  const pageId = 'dashboard'
  const pageRoute = '/plugins/' + pluginId + '/' + pageId
  const checks = []
  const check = (name) => {
    checks.push({ name, passed: true })
    console.log('PASS ' + name)
  }

  const packaged = spawnSync(
    packager,
    [
      'package',
      '--manifest',
      path.join(repo, 'examples/web_dashboard_plugin_stdio/agena-plugin.toml'),
      '--artifact',
      pluginBinary,
      '--output',
      release,
    ],
    { cwd: repo, encoding: 'utf8', timeout: 60000 },
  )
  if (packaged.error) throw packaged.error
  assert.equal(packaged.status, 0, packaged.stdout + packaged.stderr)
  const released = await fs.readdir(release)
  const archiveName = released.find((name) => name.endsWith('.tar.gz'))
  const fragmentName = released.find((name) => /^agena-plugin-release\..+\.json$/.test(name))
  assert.ok(archiveName && fragmentName, 'packager produced archive and release metadata')
  const archive = await fs.readFile(path.join(release, archiveName))
  const fragment = JSON.parse(await fs.readFile(path.join(release, fragmentName), 'utf8'))
  const artifact = fragment.artifacts[0]
  const digest = crypto.createHash('sha256').update(archive).digest('hex')
  assert.equal(artifact.sha256, digest)
  check('real release packaging with browser resources')

  let registryOrigin
  const registry = http.createServer((request, response) => {
    if (request.url === '/package.tar.gz') {
      response.writeHead(200, { 'content-type': 'application/gzip' })
      response.end(archive)
    } else if (request.url === '/index.json') {
      response.writeHead(200, { 'content-type': 'application/json' })
      response.end(
        JSON.stringify({
          version: 1,
          marketplace: { name: 'Isolated plugin Web probe' },
          plugins: [
            {
              id: pluginId,
              name: 'Plugin Web dashboard',
              versions: [
                {
                  version: fragment.version,
                  kind: 'stdio',
                  platform: artifact.target,
                  url: registryOrigin + '/package.tar.gz',
                  sha256: digest,
                  archive: artifact.archive,
                },
              ],
            },
          ],
        }),
      )
    } else {
      response.writeHead(404)
      response.end()
    }
  })
  await new Promise((resolve) => registry.listen(0, '127.0.0.1', resolve))
  registryOrigin = 'http://127.0.0.1:' + registry.address().port
  const port = await freePort()
  const origin = 'http://127.0.0.1:' + port
  const password = crypto.randomBytes(24).toString('hex')
  const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith('AGENA_')))
  Object.assign(env, {
    AGENA_DATA_DIR: data,
    AGENA_SERVER_PASSWORD: password,
    AGENA_MCP_ENABLED: 'false',
  })
  const log = await fs.open(path.join(temporary, 'server.log'), 'w')
  let server
  let browser
  let token
  let failure
  const pageErrors = []
  const commandGenerations = []
  async function request(method, route, body, authenticated = true) {
    return fetch(origin + route, {
      method,
      headers: {
        ...(body === undefined ? {} : { 'content-type': 'application/json' }),
        ...(authenticated && token ? { authorization: 'Bearer ' + token } : {}),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
  }
  async function api(method, route, body, expected = 200) {
    const response = await request(method, route, body)
    const text = await response.text()
    assert.equal(response.status, expected, method + ' ' + route + ': ' + text)
    return text ? JSON.parse(text) : undefined
  }
  async function task(start) {
    const id = start.task.id
    const finished = await until(
      () => api('GET', '/api/v1/runtime/tasks'),
      (result) => {
        const entry = result.items.find((item) => item.id === id)
        return entry && entry.status !== 'running'
      },
      'background operation ' + start.task.kind,
      120000,
    )
    const result = finished.items.find((item) => item.id === id)
    assert.equal(result.status, 'succeeded', JSON.stringify(result))
  }
  async function catalog() {
    const response = await api('GET', '/api/v1/plugins/surface')
    return (response.catalog.web_pages || []).find((entry) => entry.plugin_id === pluginId && entry.page_id === pageId)
  }
  const commandPath = (command) => '/api/v1/plugins/' + pluginId + '/web/' + pageId + '/commands/' + command

  try {
    server = spawn(
      binary,
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
      { cwd: workspace, env, stdio: ['ignore', log.fd, log.fd] },
    )
    let spawnError
    server.once('error', (error) => (spawnError = error))
    await until(
      async () => {
        if (spawnError) throw spawnError
        if (server.exitCode !== null || server.signalCode !== null)
          throw new Error('Isolated server exited: ' + (await fs.readFile(path.join(temporary, 'server.log'), 'utf8')))
        return request('GET', '/auth/session', undefined, false)
          .then((response) => [200, 401].includes(response.status))
          .catch(() => false)
      },
      Boolean,
      'isolated authenticated server',
      120000,
    )
    assert.equal((await request('GET', '/api/v1/plugins/surface', undefined, false)).status, 401)
    const login = await api('POST', '/auth/session', { password })
    token = login.token
    assert.ok(token)
    assert.equal(await catalog(), undefined)

    await task(
      await api('POST', '/api/v1/plugins/marketplace/install', {
        spec: pluginId,
        registry_id: 'plugin-web-probe',
        registry_url: registryOrigin + '/index.json',
        refresh: true,
      }),
    )
    await task(await api('POST', '/api/v1/runtime/reload'))
    let entry = await until(catalog, Boolean, 'installed page in the runtime catalog')
    assert.equal(entry.api_version, 1)
    assert.equal(entry.scope, 'instance')
    const installedConfig = JSON.parse(await fs.readFile(path.join(data, 'agena.json'), 'utf8'))
    const packageRoot = installedConfig.plugins.list[pluginId].package_root
    assert.ok(path.isAbsolute(packageRoot))
    assert.notEqual(packageRoot, path.dirname(installedConfig.plugins.list[pluginId].package.command))
    check('marketplace installation and page discovery without rebuilding the host')

    const asset = await request('GET', entry.entry_url, undefined, false)
    assert.equal(asset.status, 200)
    assert.match(asset.headers.get('content-type'), /javascript/)
    assert.equal(asset.headers.get('x-content-type-options'), 'nosniff')
    assert.ok((await asset.text()).includes('mount'))
    for (const style of entry.style_urls) {
      const response = await request('GET', style, undefined, false)
      assert.equal(response.status, 200)
      assert.match(response.headers.get('content-type'), /css/)
    }
    for (const route of [
      '/plugin-assets',
      '/plugin-assets/' + pluginId,
      '/plugin-assets/' + pluginId + '/' + entry.revision,
      '/plugin-assets/' + pluginId + '/' + entry.revision + '/missing.js',
      '/plugin-assets/' + pluginId + '/' + entry.revision + '/..%5cagena.json',
    ]) {
      assert.equal((await request('GET', route, undefined, false)).status, 404, route)
    }
    assert.equal(
      (await request('POST', commandPath('list'), { generation: entry.generation, input: {} }, false)).status,
      401,
    )
    await api('POST', commandPath('undeclared'), { generation: entry.generation, input: {} }, 403)
    await api('POST', commandPath('list'), { generation: 'retired-generation', input: {} }, 409)
    const board = await api('POST', commandPath('list'), { generation: entry.generation, input: {} })
    assert.equal(board.result.status, 'succeeded')
    assert.ok(Array.isArray(board.result.output.tasks))
    check('public versioned assets, authenticated commands, declared operations, and stale generation rejection')

    const launch = { headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] }
    if (process.env.CHROME_BIN) launch.executablePath = process.env.CHROME_BIN
    browser = await chromium.launch(launch)
    const context = await browser.newContext({
      viewport: { width: 1440, height: 1000 },
      locale: 'en-US',
      colorScheme: 'light',
    })
    const page = await context.newPage()
    page.on('pageerror', (error) => pageErrors.push(error.message))
    page.on('request', (item) => {
      if (item.url().includes('/web/' + pageId + '/commands/')) {
        try {
          commandGenerations.push(item.postDataJSON().generation)
        } catch {}
      }
      if (item.url().includes('/plugin-assets/')) {
        assert.ok(!item.url().includes(token), 'asset URL must not carry the host credential')
      }
    })
    await page.goto(origin + '/plugins')
    await page.locator('#password').fill(password)
    await page.locator('#password').press('Enter')
    // Enter through the actual primary navigation after authentication, rather
    // than coupling plugin lifecycle coverage to first-login landing policy.
    await page.locator('[data-main-nav-tab="plugins"]').waitFor({ timeout: 30000 })
    await page.locator('[data-main-nav-tab="plugins"]').click()
    await page.locator('[data-plugin-page-link]').filter({ hasText: /./ }).first().waitFor({ timeout: 30000 })
    await page.locator('[data-plugin-page-link]').first().click()
    await page.locator('[data-plugin-web-state="ready"]').waitFor({ timeout: 30000 })
    await page.locator('[data-plugin-dashboard]').waitFor()
    assert.ok(page.url().includes(pageRoute))
    check('real login, plugin directory, workspace navigation, and native module mount')

    const title = 'Browser-verified item'
    await page.locator('[data-plugin-dashboard] [data-action="new"]').first().click()
    await page.locator('[data-plugin-dashboard] [name="title"]').fill(title)
    await page.locator('[data-plugin-dashboard] [name="description"]').fill('Saved by the installed plugin backend.')
    await page.locator('[data-plugin-dashboard] [name="priority"]').selectOption('high')
    await page.locator('[data-plugin-dashboard] [data-action="save"]').click()
    await page.locator('[data-task-item]').filter({ hasText: title }).waitFor()
    // Initial UI preferences may reload the runtime after installation. The
    // out-of-band assertion must use the current catalog, just as the UI SDK does.
    entry = await until(catalog, Boolean, 'current installed page generation')
    let saved = await api('POST', commandPath('list'), { generation: entry.generation, input: {} })
    assert.equal(saved.result.status, 'succeeded')
    assert.ok(saved.result.output.tasks.some((item) => item.title === title && item.priority === 'high'))
    check('native page mutation persists through the plugin backend')
    await page.screenshot({ path: path.join(temporary, 'desktop-before-refresh.png'), fullPage: true })
    await page.reload()
    await page.locator('[data-plugin-web-state="ready"]').waitFor()
    await page.locator('[data-task-item]').filter({ hasText: title }).waitFor()
    check('custom page edits reach the plugin backend and survive deep-link refresh')
    for (const close of await page.locator('[data-sonner-toast] [data-close-button]').all()) {
      if (await close.isVisible()) await close.click()
    }
    await page.screenshot({ path: path.join(temporary, 'desktop-light.png'), fullPage: true })

    await page.evaluate(() => document.documentElement.classList.add('dark'))
    await page.waitForTimeout(150)
    await page.screenshot({ path: path.join(temporary, 'desktop-dark.png'), fullPage: true })
    await page.setViewportSize({ width: 390, height: 844 })
    await page.screenshot({ path: path.join(temporary, 'mobile.png'), fullPage: true })
    const width = await page.evaluate(() => ({
      viewport: window.innerWidth,
      page: document.documentElement.scrollWidth,
    }))
    assert.ok(width.page <= width.viewport + 1, 'mobile document must not overflow: ' + JSON.stringify(width))
    await page.locator('[data-plugin-dashboard] [data-action="new"]').first().click()
    await page.locator('[data-plugin-dashboard] [name="title"]').fill('Keyboard dismissal')
    await page.keyboard.press('Escape')
    await page.locator('[data-plugin-dashboard] [name="title"]').waitFor({ state: 'hidden' })
    check('desktop and mobile layouts, theme changes, and keyboard dismissal')

    await page.setViewportSize({ width: 1440, height: 1000 })
    const previous = entry
    await task(await api('POST', '/api/v1/runtime/reload'))
    entry = await until(catalog, (value) => value && value.generation !== previous.generation, 'new runtime generation')
    await api('POST', commandPath('list'), { generation: previous.generation, input: {} }, 409)
    await until(
      async () => commandGenerations.includes(entry.generation),
      Boolean,
      'open page remount after reload',
      30000,
    )
    await page.locator('[data-plugin-web-state="ready"]').waitFor()
    await page.locator('[data-task-item]').filter({ hasText: title }).waitFor()
    check('runtime reload retires old calls and remounts the open page with persisted data')

    const enabledPath = 'plugins.list."' + pluginId + '".enabled'
    await api('PUT', '/api/v1/settings', { path: enabledPath, value: false, reload: true })
    await until(catalog, (value) => !value, 'disabled plugin removed from catalog', 60000)
    await page.locator('[data-plugin-web-state="unavailable"]').waitFor({ timeout: 30000 })
    assert.equal((await request('GET', entry.entry_url, undefined, false)).status, 404)
    await page.locator('[data-plugin-dashboard]').waitFor({ state: 'detached' })
    await api('PUT', '/api/v1/settings', { path: enabledPath, value: true, reload: true })
    entry = await until(catalog, Boolean, 're-enabled plugin catalog', 60000)
    await page.locator('[data-plugin-web-state="ready"]').waitFor({ timeout: 30000 })
    await page.locator('[data-task-item]').filter({ hasText: title }).waitFor()
    check('disable and re-enable clean up the mounted page and restore its data')

    await task(await api('POST', '/api/v1/plugins/marketplace/uninstall', { plugin_id: pluginId }))
    await task(await api('POST', '/api/v1/runtime/reload'))
    await until(catalog, (value) => !value, 'uninstalled page removed from catalog')
    await page.locator('[data-plugin-web-state="unavailable"]').waitFor({ timeout: 30000 })
    assert.equal((await request('GET', entry.entry_url, undefined, false)).status, 404)
    assert.deepEqual(pageErrors, [], 'browser JavaScript errors')
    check('uninstall revokes page discovery and resources without browser errors')

    // The all-plugin hub shows builtins even when they export no Web assets.
    await page.goto(origin + '/plugins')
    await page.locator('[data-plugin-entry="agena.telegram"]').waitFor({ timeout: 30000 })
    assert.ok((await api('GET', '/api/v1/plugins')).items.some((item) => item.plugin_id === 'agena.telegram'))
    await page.locator('[data-plugin-entry="agena.telegram"]').click()
    await page.locator('[data-plugin-web-state="telegram"]').waitFor({ timeout: 30000 })
    await page.locator('[data-testid="telegram-connection"]').waitFor()
    assert.ok(page.url().includes('/plugins/agena.telegram/manage'))
    check('all-plugin directory discovers Telegram without surface.web and opens native manager')
  } catch (error) {
    failure = error
    console.error(error.stack || error)
    const failedPage = browser?.contexts()[0]?.pages()[0]
    if (failedPage && !failedPage.isClosed()) {
      await failedPage.screenshot({ path: path.join(temporary, 'failure.png'), fullPage: true }).catch(() => {})
      const snapshot = await failedPage
        .evaluate(() => ({
          url: location.href,
          text: document.body.innerText,
          pluginPages: [...document.querySelectorAll('[data-plugin-web-page]')].map((element) => ({
            state: element.getAttribute('data-plugin-web-state'),
            visible: element.getClientRects().length > 0,
            text: element.innerText,
          })),
        }))
        .catch(() => null)
      if (snapshot)
        await fs.writeFile(path.join(temporary, 'browser-failure.json'), JSON.stringify(snapshot, null, 2) + '\n')
    }
  } finally {
    await browser?.close().catch(() => {})
    await terminate(server)
    registry.closeAllConnections()
    await new Promise((resolve) => registry.close(resolve))
    await log.close()
    await fs.writeFile(
      path.join(temporary, 'report.json'),
      JSON.stringify({ passed: !failure, checks, pageErrors, error: failure?.message || null }, null, 2) + '\n',
    )
    console.log('Plugin Web probe artifacts: ' + temporary)
  }
  if (failure) throw failure
}

main().catch((error) => {
  console.error(error.message || error)
  process.exitCode = 1
})
