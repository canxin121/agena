// Real workbench components and routes in Chromium, with same-origin fixtures only.
const assert = require('node:assert/strict')
const path = require('node:path')
const { mkdir, mkdtemp, rm } = require('node:fs/promises')
const { pathToFileURL } = require('node:url')
const {
  fileName,
  project,
  sessionLabels,
  terminalPath,
  terminalStateEvents,
  previewPath,
  workspaceApi,
  writeTerminalStateStream,
} = require('./fixtures/workbench-reuse-api.cjs')
const { channelAssignmentsApi, promptListsApi, sessionStorageApi } = require('./fixtures/workbench-list-reuse-api.cjs')
const { runListChecks } = require('./probe-workbench-list-reuse.cjs')

const web = path.resolve(__dirname, '..')
const screenshotDir = process.env.PROBE_SCREENSHOT_DIR
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright')

async function createProbeServer(cacheDir) {
  const { createServer } = await import(pathToFileURL(require.resolve('vite')).href)
  return createServer({
    root: web,
    configFile: path.join(web, 'vite.config.ts'),
    // Fixtures are served by this middleware, so include their entry points in
    // Vite's scan and keep concurrent dev/probe servers from invalidating this cache.
    cacheDir,
    optimizeDeps: {
      entries: ['index.html', 'scripts/fixtures/workbench-reuse.ts', 'scripts/fixtures/workbench-list-reuse.ts'],
    },
    logLevel: 'error',
    server: { host: '127.0.0.1', port: 0 },
    plugins: [
      {
        name: 'workbench-reuse-fixture',
        configureServer(dev) {
          dev.middlewares.use(async (req, res, next) => {
            if (req.url.split('?')[0] === terminalStateEvents) return writeTerminalStateStream(res)
            // Never let a missing fixture fall through to Vite's backend proxy.
            if (/^\/(api|auth|health|plugin-assets)(\/|\?|$)/.test(req.url)) {
              res.statusCode = 404
              return res.end('Missing workbench fixture')
            }
            const entry = req.url.startsWith('/__workbench-lists')
              ? 'workbench-list-reuse.ts'
              : req.url.startsWith('/__workbench-reuse')
                ? 'workbench-reuse.ts'
                : ''
            if (!entry) return next()
            try {
              res.setHeader('Content-Type', 'text/html')
              res.end(
                await dev.transformIndexHtml(
                  req.url,
                  `<!doctype html><html lang="en"><head>
                <meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
                <title>Workbench reuse probe</title></head><body><div id="app"></div>
                <script type="module" src="/scripts/fixtures/${entry}"></script></body></html>`,
                ),
              )
            } catch (error) {
              next(error)
            }
          })
        },
      },
    ],
  })
}

async function main() {
  const cacheDir = await mkdtemp(path.join(web, 'node_modules', '.vite-workbench-'))
  let server
  let browser
  let currentPage
  const errors = []
  try {
    server = await createProbeServer(cacheDir)
    await server.listen()
    const origin = 'http://127.0.0.1:' + server.httpServer.address().port
    browser = await chromium.launch({
      executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
      headless: true,
      args: ['--no-sandbox'],
    })
    async function openPage(url, { viewport = { width: 1280, height: 850 }, narrow = false, api, storage } = {}) {
      const page = await browser.newPage({ viewport, hasTouch: narrow, isMobile: narrow, serviceWorkers: 'block' })
      currentPage = page
      page.setDefaultTimeout(10000)
      page.on('pageerror', (error) => errors.push(`${url}: ${error.message}`))
      page.on('console', (message) => {
        if (message.type() === 'warning' && /teleport/i.test(message.text())) errors.push(`${url}: ${message.text()}`)
      })
      await page.addInitScript(
        (items) => {
          for (const [key, value] of Object.entries(items)) localStorage.setItem(key, value)
        },
        { 'agena.settings.ui-prefs': JSON.stringify({ browserNotificationsPrompted: true }), ...storage },
      )
      await page.route('**/*', (route) => {
        const target = new URL(route.request().url())
        if (target.origin !== origin) return route.abort()
        if (target.pathname === terminalStateEvents) return route.continue()
        if (/^\/(api|auth|health|plugin-assets)(\/|$)/.test(target.pathname)) {
          return api?.(route, target) ?? route.abort()
        }
        return route.continue()
      })
      await page.goto(origin + url, { waitUntil: 'domcontentloaded' })
      return page
    }

    if (screenshotDir) await mkdir(screenshotDir, { recursive: true })
    for (const kind of ['terminal', 'preview']) {
      await checkDock(openPage, kind, false)
      await checkDock(openPage, kind, true)
    }
    console.log('Terminal and Preview Dock pickers passed on desktop and touch.')
    await checkAssistant(openPage)
    await checkGitTabs(openPage)
    console.log('Assistant and Git tab navigation and content passed.')
    await checkShell(openPage)
    await checkEmbedded(openPage)
    console.log('Hosted, standalone, and embedded workspace sidebars passed.')
    for (const narrow of [false, true]) {
      await checkStorageParent(openPage, narrow)
      await checkAssignmentsParent(openPage, narrow)
    }
    console.log('Storage and Channel Assignments passed in the real Settings layout.')
    await checkSystemPromptParent(openPage)
    console.log('System Prompt inheritance and read-only preview passed in the real Settings layout.')
    await runListChecks(openPage)
    assert.deepEqual(errors, [])
    console.log('Workbench reuse and keyboard navigation passed in Chromium.')
  } catch (error) {
    if (currentPage && !currentPage.isClosed()) {
      console.error('Failed browser page:', currentPage.url())
      await saveScreenshot(currentPage, 'failure')?.catch(() => {})
    }
    if (errors.length) console.error(errors)
    throw error
  } finally {
    await browser?.close()
    await server?.close()
    await rm(cacheDir, { recursive: true, force: true })
  }
}

const focus = (locator) => locator.and(locator.page().locator(':focus')).waitFor({ state: 'attached' })
const button = (scope, name) => scope.getByRole('button', { name, exact: typeof name === 'string' })
const box = async (locator) => {
  const rectangle = await locator.boundingBox()
  assert.ok(rectangle, 'Expected element to have measurable bounds')
  return rectangle
}

const phoneViewport = { width: 390, height: 844 }
const fixture = (name, embedded = false) => `/__workbench-reuse?case=${name}${embedded ? '&embedded' : ''}`
const saveScreenshot = (page, name) =>
  screenshotDir ? page.screenshot({ path: path.join(screenshotDir, `${name}.png`), animations: 'disabled' }) : undefined

async function checkDock(openPage, kind, narrow) {
  const labels = sessionLabels(kind)
  const terminal = kind === 'terminal'
  const statePath = terminal ? `${terminalPath}/state` : previewPath
  const api = workspaceApi()
  const requests = []
  const page = await openPage(fixture(`${kind}-dock`), {
    narrow,
    viewport: narrow ? phoneViewport : undefined,
    storage: terminal ? undefined : { 'studio.ui.workspace-preview.active-session-id': labels[0] },
    api: (route, url) => {
      if (url.pathname === statePath) requests.push(route.request().method())
      return api(route, url)
    },
  })
  // The modal sheet hides the trigger from the accessibility tree while open.
  const trigger = page.locator(`button[aria-haspopup="dialog"][aria-label^="Sessions: ${kind}-"]`)
  const expanded = (value) => trigger.and(page.locator(`[aria-expanded="${value}"]`))
  const menu = page.getByRole('dialog', { name: 'Sessions', exact: true })
  const search = menu.locator('[data-option-menu-search]')
  const items = menu.locator('[data-option-menu-item]')
  const item = (index) => items.filter({ hasText: labels[index] })
  const refresh = button(page.locator('#app'), terminal ? 'Refresh panel' : 'Refresh preview')
  await expanded(false).waitFor()

  // A long active name must truncate without pushing any Dock actions off the row.
  const label = trigger.locator('.truncate')
  assert.ok(
    await label.evaluate((el) => el.scrollWidth > el.clientWidth && getComputedStyle(el).textOverflow === 'ellipsis'),
  )
  const triggerBox = await box(trigger)
  const actions = await refresh.locator('..').getByRole('button').all()
  let right = triggerBox.x + triggerBox.width
  for (const action of actions) {
    const bounds = await box(action)
    assert.ok(
      bounds.x >= right && bounds.width >= 24 && bounds.x + bounds.width <= page.viewportSize().width,
      `${kind} control overlaps another control or leaves the viewport`,
    )
    right = bounds.x + bounds.width
  }

  await trigger.press('ArrowDown')
  await expanded(true).waitFor()
  await focus(item(0))
  assert.equal(await items.count(), 18)
  const menuBox = await box(menu)
  if (narrow) {
    assert.equal(await menu.getAttribute('data-keyboard-overlay'), 'true')
    await button(menu, 'Close').waitFor()
    assert.ok(menuBox.x >= -1 && menuBox.x + menuBox.width <= phoneViewport.width + 1)
  } else {
    assert.ok(Math.abs(menuBox.x - triggerBox.x) <= 2, 'Desktop menu is not aligned to its trigger')
    assert.ok(
      Math.abs(menuBox.y - (triggerBox.y + triggerBox.height + 8)) <= 2,
      'Desktop menu is not below its trigger',
    )
  }
  await saveScreenshot(page, `${kind}-picker-${narrow ? 'touch' : 'desktop'}`)
  await page.keyboard.press('Escape')
  await expanded(false).waitFor()
  await menu.waitFor({ state: 'hidden' })
  await focus(trigger)

  await trigger.press('ArrowUp')
  await focus(item(17))
  assert.ok(
    await item(17).evaluate((el) => {
      const scroll = el.closest('.overflow-auto')
      return (
        scroll && scroll.scrollTop > 0 && el.getBoundingClientRect().bottom <= scroll.getBoundingClientRect().bottom + 1
      )
    }),
    'Opening at the last option should scroll it into view',
  )
  if (terminal && !narrow) {
    const refreshed = page.waitForResponse(
      (response) => new URL(response.url()).pathname === statePath && response.ok(),
    )
    await button(menu, 'Refresh').click()
    await refreshed
    await item(0).waitFor()
  }
  await button(menu, 'Next page').click()
  await item(19).waitFor()
  assert.equal(await items.count(), 2)
  await search.fill(labels[4])
  await item(4).waitFor()
  assert.equal(await items.count(), 1, 'Searching from page two should reset to the matching page')
  await search.press('Enter')
  await trigger.and(page.getByRole('button', { name: `Sessions: ${labels[4]}`, exact: true })).waitFor()
  await expanded(false).waitFor()
  await focus(trigger)
  if (terminal) {
    assert.ok(requests.includes('PUT'), 'Terminal selection did not persist state')
  } else {
    const refreshed = page.waitForResponse(
      (response) => new URL(response.url()).pathname === statePath && response.ok(),
    )
    await refresh.click()
    await refreshed
    await trigger.and(page.getByRole('button', { name: `Sessions: ${labels[4]}`, exact: true })).waitFor()
  }
  await trigger.press('ArrowDown')
  await focus(item(0))
  assert.equal(await item(4).getAttribute('aria-current'), 'true')
  if (narrow) await button(menu, 'Close').tap()
  else await page.keyboard.press('Escape')
  await focus(trigger)
  await page.close()
}

async function assertTabRelations(page, tablist, current) {
  const tabs = await tablist.getByRole('tab').all()
  assert.ok(tabs.length > 1)
  const currentId = await current.getAttribute('id')
  const ids = new Set()
  for (const tab of tabs) {
    const id = await tab.getAttribute('id')
    const controls = await tab.getAttribute('aria-controls')
    assert.ok(id && controls && !ids.has(controls), 'Each tab needs its own labelled panel')
    ids.add(controls)
    const active = id === currentId
    assert.equal(await tab.getAttribute('aria-selected'), String(active))
    assert.equal(await tab.getAttribute('tabindex'), active ? '0' : '-1')
    const panel = page.locator(`[id="${controls}"]`)
    await panel.waitFor({ state: active ? 'visible' : 'attached' })
    assert.equal(await panel.getAttribute('aria-labelledby'), id)
    assert.equal(await panel.getAttribute('role'), 'tabpanel')
    assert.equal(await panel.isVisible(), active)
  }
}

async function checkAssistant(openPage) {
  const page = await openPage(fixture('assistant'), { viewport: phoneViewport, narrow: true, api: workspaceApi() })
  const tablist = page.getByRole('tablist', { name: 'Assistant' })
  const profile = page.getByTestId('assistant-tab-profile')
  const memory = page.getByTestId('assistant-tab-memory')
  const reminders = page.getByTestId('assistant-tab-reminders')
  const timezone = page.getByTestId('assistant-timezone')
  await timezone.waitFor()
  await assertTabRelations(page, tablist, profile)
  await timezone.fill('Europe/Paris')
  await profile.press('ArrowRight')
  await focus(memory)
  await assertTabRelations(page, tablist, memory)
  await page.getByText('No saved memories in this workspace.').waitFor()
  await memory.press('End')
  await focus(reminders)
  await assertTabRelations(page, tablist, reminders)
  await page.getByText('No reminders scheduled.').waitFor()
  await reminders.press('Home')
  await focus(profile)
  await assertTabRelations(page, tablist, profile)
  assert.equal(await timezone.inputValue(), 'Europe/Paris')
  await page.evaluate(() => {
    document.documentElement.dir = 'rtl'
  })
  await profile.press('ArrowLeft')
  await focus(memory)
  await assertTabRelations(page, tablist, memory)
  await page.keyboard.press('Tab')
  await focus(page.getByRole('tabpanel', { name: 'Memory' }))
  await page.close()
}

const sidebar = (scope, panel) => scope.locator(`[data-workspace-sidebar="${panel}"]`)
const resizeHandle = (scope) => scope.getByRole('separator', { name: 'Resizing sidebar…', exact: true })
const fileRow = (scope) => scope.getByText(fileName, { exact: true })
const changesRow = (scope) => scope.locator('[data-keyboard-item="changed.txt"]')
const gitTablist = (scope) => scope.getByRole('tablist', { name: 'Source Control' })

async function waitForWidth(locator, width) {
  const element = await locator.elementHandle()
  await locator
    .page()
    .waitForFunction(([el, expected]) => Math.abs(el.getBoundingClientRect().width - expected) < 1, [element, width])
  await element.dispose()
}

async function checkGitTabs(openPage) {
  const page = await openPage(fixture('git'), { api: workspaceApi() })
  const side = sidebar(page, 'git')
  const handle = resizeHandle(side)
  await handle.press('Home')
  await waitForWidth(side, 280)
  await page.evaluate(() => {
    document.documentElement.dir = 'rtl'
  })
  await handle.press('ArrowLeft')
  await waitForWidth(side, 296)
  const tablist = gitTablist(side)
  const changes = tablist.getByRole('tab', { name: /^Changes/ })
  const history = tablist.getByRole('tab', { name: 'History', exact: true })
  await changesRow(side).waitFor()
  await assertTabRelations(page, tablist, changes)
  assert.match(await changes.innerText(), /\b1\b/)
  await changes.press('ArrowLeft')
  await focus(history)
  await assertTabRelations(page, tablist, history)
  const panel = side.getByRole('tabpanel', { name: 'History' })
  const commit = button(panel, /Fixture history commit/)
  await commit.click()
  await button(panel, /changed\.txt/).waitFor()
  await assertTabRelations(page, tablist, history)
  await button(panel, 'History').click()
  await commit.waitFor()
  await history.press('Home')
  await focus(changes)
  await assertTabRelations(page, tablist, changes)
  await changesRow(side).waitFor()
  await page.close()
}

async function checkShell(openPage) {
  const page = await openPage(`/files?directory=${encodeURIComponent(project)}`, { api: workspaceApi() })
  const host = page.locator('#workspace-sidebar-panel-host')
  const shell = page.locator('aside:has(> #workspace-sidebar-host)')
  const rail = (panel) => shell.locator(`[data-main-nav-tab="${panel}"]`)
  const files = sidebar(host, 'files')
  const handle = resizeHandle(shell)
  const header = page.locator('header')
  await fileRow(files).waitFor()
  assert.equal(await resizeHandle(page).count(), 1)
  assert.ok(
    await fileRow(files).evaluate(
      (el) => el.scrollWidth > el.clientWidth && getComputedStyle(el).textOverflow === 'ellipsis',
    ),
  )
  await fileRow(files).evaluate((el) => {
    el.dataset.probeRetained = 'true'
  })
  await handle.press('Home')
  await waitForWidth(shell, 280)
  await handle.press('ArrowRight')
  await waitForWidth(shell, 296)
  const start = await box(handle)
  await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2)
  await page.mouse.down()
  await page.mouse.move(start.x + start.width / 2 + 50, start.y + start.height / 2)
  await waitForWidth(shell, 346)
  assert.equal(await handle.getAttribute('aria-valuenow'), '296', 'Dragging should preview, not commit, shell width')
  await page.mouse.up()
  assert.equal(await handle.getAttribute('aria-valuenow'), '346')
  await saveScreenshot(page, 'files-shell-desktop')
  await button(shell, 'Collapse sidebar').click()
  await waitForWidth(shell, 76)
  await files.waitFor({ state: 'hidden' })
  await button(shell, 'Expand sidebar').click()
  await waitForWidth(shell, 346)
  await fileRow(files).waitFor()

  await page.setViewportSize(phoneViewport)
  await host.waitFor({ state: 'detached' })
  const inlineFiles = sidebar(page, 'files')
  await inlineFiles.waitFor({ state: 'hidden' })
  await button(header, 'Open files panel').click()
  await fileRow(inlineFiles).and(page.locator('[data-probe-retained="true"]')).waitFor()
  assert.ok((await box(inlineFiles)).width >= phoneViewport.width - 2)
  await saveScreenshot(page, 'files-shell-compact')
  await fileRow(inlineFiles).click()
  await inlineFiles.waitFor({ state: 'hidden' })
  const image = page.getByRole('img', { name: fileName, exact: true })
  await image.waitFor()
  await image.evaluate((el) => el.decode())
  await button(page, 'Back').click()
  await fileRow(inlineFiles).waitFor()
  await image.waitFor({ state: 'hidden' })
  await page.setViewportSize({ width: 1280, height: 850 })
  await fileRow(files).and(page.locator('[data-probe-retained="true"]')).waitFor()

  const actionMenu = page.locator('[data-workspace-tab-main="files"]').getByRole('button', { name: /^Actions for / })
  await actionMenu.click()
  await button(page.getByRole('dialog'), 'Duplicate in a group to the right').click()
  const pane = (route) => page.locator(`[data-workspace-pane-window][data-keyboard-route="/${route}"]:visible`)
  await pane('files').nth(1).waitFor()
  await rail('git').click()
  await gitTablist(sidebar(host, 'git')).waitFor()
  await pane('git').waitFor()
  await pane('files').focus()
  await fileRow(files).waitFor()
  assert.equal(await host.locator('[data-workspace-sidebar]:visible').count(), 1)
  assert.equal(await sidebar(host, 'git').count(), 0)
  await pane('git').focus()
  await gitTablist(sidebar(host, 'git')).waitFor()
  assert.equal(await host.locator('[data-workspace-sidebar]:visible').count(), 1)
  for (const panel of ['terminal', 'preview', 'settings']) {
    await rail(panel).click()
    await sidebar(host, panel).waitFor()
    assert.equal(await host.locator('[data-workspace-sidebar]:visible').count(), 1)
    assert.equal(await resizeHandle(page).count(), 1, `${panel} should not add a local resize handle`)
  }
  await page.close()

  // A direct Files route is valid even before choosing a project.
  const empty = await openPage('/files', { api: workspaceApi('') })
  await sidebar(empty.locator('#workspace-sidebar-panel-host'), 'files').getByText('No Project Selected').waitFor()
  await empty.setViewportSize(phoneViewport)
  await button(empty.locator('header'), 'Open files panel').click()
  await empty.getByText('Choose a project folder first, then the file tree will appear here.').waitFor()
  await empty.close()

  const compact = await openPage(`/git?directory=${encodeURIComponent(project)}`, {
    narrow: true,
    viewport: phoneViewport,
    api: workspaceApi(),
    storage: { 'studio.directory.current-directory-by-window': JSON.stringify({ 'window-default': project }) },
  })
  const git = sidebar(compact, 'git')
  const openSourceControl = button(compact.locator('header'), 'Open source control panel')
  await openSourceControl.click()
  await gitTablist(git).getByRole('tab', { name: 'History' }).click()
  const panel = git.getByRole('tabpanel', { name: 'History' })
  await button(panel, /Fixture history commit/).click()
  await button(panel, /changed\.txt/).click()
  await git.waitFor({ state: 'hidden' })
  await compact.getByText('changed.txt', { exact: true }).filter({ visible: true }).first().waitFor()
  await button(compact, 'Back').click()
  await button(git.getByRole('tabpanel', { name: 'History' }), /changed\.txt/).waitFor()
  await compact.close()
}

async function checkEmbedded(openPage) {
  const options = { narrow: true, viewport: phoneViewport, api: workspaceApi() }
  const filesPage = await openPage(fixture('files', true), options)
  const files = sidebar(filesPage, 'files')
  const image = filesPage.getByRole('img', { name: fileName, exact: true })
  await fileRow(files).click()
  await files.waitFor({ state: 'hidden' })
  await image.waitFor()
  await button(filesPage, 'Back').click()
  await fileRow(files).waitFor()
  await image.waitFor({ state: 'hidden' })
  await filesPage.close()

  const gitPage = await openPage(fixture('git', true), options)
  const git = sidebar(gitPage, 'git')
  await changesRow(git).click()
  await git.waitFor({ state: 'hidden' })
  await gitPage.getByText('changed.txt', { exact: true }).filter({ visible: true }).first().waitFor()
  await button(gitPage, 'Source Control').click()
  await changesRow(git).waitFor()
  const tabs = gitTablist(git)
  const changes = tabs.getByRole('tab', { name: /^Changes/ })
  const history = tabs.getByRole('tab', { name: 'History' })
  await changes.press('End')
  await focus(history)
  await history.press('Home')
  await focus(changes)
  await changesRow(git).waitFor()
  assert.equal(await resizeHandle(gitPage).count(), 0)
  await gitPage.close()

  const settingsPage = await openPage(fixture('settings'), { api: workspaceApi() })
  const settings = sidebar(settingsPage, 'settings')
  await settings.waitFor()
  assert.equal(await resizeHandle(settingsPage).count(), 1)
  await resizeHandle(settings).press('End')
  await waitForWidth(settings, 520)
  await settingsPage.close()
}

const settingsViewport = (narrow) => (narrow ? { ...phoneViewport, height: 650 } : { width: 1280, height: 540 })

async function checkSettingsGrid(pane, list, detail, narrow) {
  assert.ok(await pane.evaluate((el) => el.scrollWidth <= el.clientWidth + 1), 'Settings overflows horizontally')
  const detailBox = await box(detail)
  if (narrow) {
    await list.waitFor({ state: 'hidden' })
    assert.ok(Math.abs(detailBox.width - (await box(pane)).width) < 40, 'Stacked detail is not full width')
  } else {
    const listBox = await box(list)
    assert.ok(listBox.x + listBox.width < detailBox.x, 'The desktop list and detail are not side by side')
    assert.ok(Math.abs(listBox.y - detailBox.y) <= 1, 'The desktop list and detail do not share a top edge')
  }
}

async function reachInSettings(pane, control) {
  await control.scrollIntoViewIfNeeded()
  const bounds = await box(control)
  const scrollport = await box(pane)
  assert.ok(bounds.y >= scrollport.y - 1 && bounds.y + bounds.height <= scrollport.y + scrollport.height + 1)
  assert.ok(await pane.evaluate((el) => el.scrollTop > 0), 'The outer Settings content did not scroll')
}

async function checkStorageParent(openPage, narrow) {
  const api = sessionStorageApi({ fallback: workspaceApi() })
  const page = await openPage('/settings/runtime-session?view=storage&targetSessionId=144&storageBot=bot-1', {
    narrow,
    viewport: settingsViewport(narrow),
    api,
  })
  const pane = page.locator('main.settings-pane')
  const storage = pane.getByTestId('session-storage')
  const list = storage.locator('aside[aria-label="Sessions"]')
  const rows = list.locator('[data-keyboard-item]')
  const detail = storage.locator('[data-settings-detail]')
  const heading = detail.locator('[data-settings-detail-heading]')
  await heading.filter({ hasText: /^Session 44$/ }).waitFor()
  assert.equal(await storage.getByLabel('AI Bot', { exact: true }).inputValue(), 'bot-1')
  assert.ok(api.state.listRequests.some(({ query }) => query.bot === 'bot-1' && query.offset === '0'))
  assert.equal(await rows.filter({ hasText: 'Session 44' }).count(), 0, 'Deep link must work outside the first page')
  await checkSettingsGrid(pane, list, detail, narrow)
  if (narrow) {
    await focus(heading)
    await button(detail, 'Sessions').press('Enter')
    await focus(list.locator('[data-settings-list-heading]'))
  }
  await rows.first().press('End')
  await focus(rows.last())
  const listBody = list.locator('.settings-list')
  assert.ok(
    await listBody.evaluate((el) => el.scrollTop > 0),
    'The capped Settings list did not scroll to its last row',
  )
  if (!narrow) assert.equal(await heading.textContent(), 'Session 44', 'Browsing activated another session')
  await rows.last().press('Enter')
  await heading.filter({ hasText: /^Session 40$/ }).waitFor()
  await page.waitForURL((url) => url.searchParams.get('targetSessionId') === '140')
  if (narrow) await focus(heading)

  await detail.locator('summary').filter({ hasText: 'Managed files' }).click()
  await detail.getByText('artifact-02.bin', { exact: true }).waitFor()
  await reachInSettings(pane, storage.locator('summary').filter({ hasText: 'Shared runtime files' }))
  await saveScreenshot(page, `settings-storage-${narrow ? 'narrow' : 'desktop'}`)
  if (narrow) {
    await button(detail, 'Sessions').click()
    await focus(rows.last())
  }
  await page.close()
}

async function checkAssignmentsParent(openPage, narrow) {
  const api = channelAssignmentsApi({ fallback: workspaceApi() })
  api.state.participants['room-alpha'].push(
    ...Array.from({ length: 10 }, (_, index) => ({
      actor: `fixture-member-${index}`,
      display_name: `Participant ${index}`,
      first_seen_at_ms: 1,
      last_seen_at_ms: 10,
    })),
  )
  const query = new URLSearchParams({
    view: 'assignments',
    scope: 'global',
    channelSource: '5',
    assignment: JSON.stringify(['telegram', 'tenant-1', 'acct-1', 'room-alpha', 'ada']),
  })
  const page = await openPage(`/settings/permissions?${query}`, { narrow, viewport: settingsViewport(narrow), api })
  const pane = page.locator('main.settings-pane')
  const assignments = pane.locator('[data-channel-assignments]')
  const list = assignments.locator('aside[aria-label="Chats and channels"]')
  const room = list.locator('[data-assignment-room="room-alpha"]')
  const detail = assignments.locator('[data-settings-detail]')
  const heading = detail.locator('[data-settings-detail-heading]')
  const profile = detail.locator('[data-assignment-room-profile]')
  const memberSearch = detail.getByRole('textbox', { name: 'Search members by name or ID' })
  await heading.filter({ hasText: /^Alpha room$/ }).waitFor()
  await detail.locator('[data-assignment-member="ada"]:checked').waitFor()
  assert.equal(await memberSearch.inputValue(), 'ada')
  await checkSettingsGrid(pane, list, detail, narrow)
  if (narrow) await focus(heading)
  await memberSearch.fill('')
  await button(detail, 'Load more participants').click()
  await detail.getByText('Participant 9', { exact: true }).waitFor()
  await profile.selectOption('reviewer')

  if (narrow) {
    const back = button(detail, 'Chats and channels')
    const otherRoom = list.locator('[data-assignment-room="room-beta"]')
    const discard = page.getByRole('dialog', { name: 'Discard unsaved changes?' })
    await back.press('Enter')
    await focus(list.locator('[data-settings-list-heading]'))
    await room.press('ArrowDown')
    await focus(otherRoom)
    assert.equal(await room.getAttribute('aria-pressed'), 'true', 'Browsing activated another room')
    assert.equal(await discard.count(), 0)
    await otherRoom.press('Enter')
    await button(discard, 'Keep editing').click()
    await discard.waitFor({ state: 'hidden' })
    await room.press('Enter')
    await focus(heading)
    assert.equal(await profile.inputValue(), 'reviewer', 'Back or cancelling a room switch lost the draft')
    await back.press('Enter')
    await focus(room)
    await room.press('Enter')
    await focus(heading)
  }
  const save = button(detail.locator('[data-assignment-save]'), 'Save changes')
  await pane.evaluate((el) => el.scrollTo(0, 0))
  await reachInSettings(pane, save)
  await saveScreenshot(page, `settings-assignments-${narrow ? 'narrow' : 'desktop'}`)
  await save.click()
  await assignments.getByText('Saved', { exact: true }).waitFor()
  assert.equal(api.writes.length, 1)
  assert.equal(
    api.writes[0].value.conversations.find((entry) => entry.resource === 'room-alpha')?.default_profile_id,
    'reviewer',
  )
  await page.close()
}

async function checkSystemPromptParent(openPage) {
  const api = promptListsApi({
    fallback: workspaceApi(),
    state: { sessionPrompts: { mode: 'append', order: [], disabled: [], inline: [] } },
  })
  api.state.global.order = ['agena.system.evidence', 'agena.system.identity', 'saved.alpha', 'saved.beta']
  const page = await openPage('/settings/runtime-session?view=system-prompt&scope=session&targetSessionId=42', { api })
  const pane = page.locator('main.settings-pane')
  await pane.locator('[data-permission-session-picker] button').getByText('Fixture session', { exact: true }).waitFor()
  const editor = pane.locator('[data-system-prompt]')
  const row = (id) => editor.locator(`[data-system-prompt-entry="${id}"]`)
  const rows = editor.locator('[data-system-prompt-entry]')
  await rows.first().waitFor()
  assert.deepEqual(
    await rows.evaluateAll((elements) => elements.map((el) => el.dataset.systemPromptEntry)),
    ['agena.system.evidence', 'agena.system.identity'],
    'The edit list inherits the required order, not the optional parent entries',
  )

  const add = editor.locator('[data-system-prompt-add] button')
  const picker = page.getByRole('dialog', { name: 'Add prompt', exact: true })
  await add.press('Enter')
  await picker.locator('[data-option-menu-search]').fill('Workspace note')
  await picker.locator('[data-option-menu-item]').filter({ hasText: 'Workspace note' }).press('Enter')
  await row('saved.workspace').waitFor()
  await button(editor, 'Save changes').waitFor()

  await button(editor, 'Preview saved prompt').click()
  await row('saved.alpha').waitFor()
  await row('saved.alpha').locator('[data-keyboard-primary]').press('Enter')
  await editor.locator('[data-settings-detail] pre').filter({ hasText: 'Alpha guidance body.' }).waitFor()
  assert.equal(await row('saved.workspace').count(), 0, 'The saved preview must not include the unsaved entry')
  const editActions = editor.locator('[data-system-prompt-add], [aria-label="Remove"], [aria-label="Move up"]')
  assert.equal(await editActions.count(), 0)
  assert.equal(await button(editor, 'Save changes').count(), 0)
  assert.ok(
    api.requests.some(({ method, pathname }) => method === 'GET' && pathname === '/api/v1/sessions/42/prompts/preview'),
  )
  assert.equal(api.writes.length, 0)
  await button(editor, 'Back to prompt order').click()
  await row('saved.workspace').waitFor()
  await button(editor, 'Save changes').waitFor()
  await saveScreenshot(page, 'settings-system-prompt-session')
  await page.close()
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
