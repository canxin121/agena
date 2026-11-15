// Plugin cards, detail navigation, draft protection and responsive layout against isolated fixtures.
// Run with Node. Optional: PLAYWRIGHT_MODULE, CHROME_BIN, PROBE_REPORT, PROBE_SCREENSHOT_DIR.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const os = require('node:os')
const path = require('node:path')
const { createRequire } = require('node:module')
const { pathToFileURL } = require('node:url')

async function main() {
  const web = path.resolve(process.env.AGENA_WEB_ROOT || path.join(__dirname, '..'))
  const requireFromWeb = createRequire(path.join(web, 'package.json'))
  const { chromium } = require(process.env.PLAYWRIGHT_MODULE || requireFromWeb.resolve('playwright'))
  const { createServer } = await import(pathToFileURL(requireFromWeb.resolve('vite')).href)
  const temporary = await fs.mkdtemp(path.join(web, '.plugin-cards-probe-'))
  const reportPath = process.env.PROBE_REPORT || path.join(os.tmpdir(), 'agena-plugin-cards-report.json')
  const screenshotDir = process.env.PROBE_SCREENSHOT_DIR || path.join(os.tmpdir(), 'agena-plugin-cards-screenshots')
  const report = {
    passed: false,
    checks: [],
    browserErrors: [],
    externalRequests: [],
    unexpectedApiRequests: [],
    requests: [],
  }
  let browser, server, page
  const check = (condition, description) => {
    assert(condition, description)
    report.checks.push(description)
    console.log('PASS ' + description)
  }
  const statuses = [
    {
      plugin_id: 'agena.telegram',
      kind: 'builtin',
      state: 'running',
      restart_count: 0,
      summary: 'Connect conversations and manage Telegram integrations.',
    },
    {
      plugin_id: 'fixture.board',
      kind: 'stdio',
      state: 'running',
      restart_count: 0,
      summary: 'A project dashboard with history and interactive reports.',
    },
    {
      plugin_id: 'fixture.stopped',
      kind: 'stdio',
      state: 'stopped',
      restart_count: 0,
      summary: 'An installed application that is currently stopped.',
    },
    {
      plugin_id: 'fixture.very-long-plugin-identifier-for-responsive-layout-and-keyboard-navigation',
      kind: 'builtin',
      state: 'failed',
      restart_count: 0,
      summary:
        'A long description with enough information to verify wrapping on small screens and readable card actions.',
    },
  ]
  const webPage = (pluginId, pageId, title, scope = 'instance') => ({
    plugin_id: pluginId,
    page_id: pageId,
    title,
    api_version: 1,
    scope,
    entry_url: '/plugin-assets/' + pluginId + '/r1/index.js',
    style_urls: [],
    commands: [],
    settings: true,
    revision: 'r1',
    generation: 'g1',
  })
  const pages = [
    webPage('fixture.board', 'dashboard', 'Dashboard'),
    webPage('fixture.board', 'history', 'History', 'session'),
    webPage('fixture.stopped', 'dashboard', 'Dashboard'),
  ]
  const contract = {
    version: 1,
    root: {
      id: 'root',
      path: '',
      title: 'Fixture settings',
      kind: 'object',
      fields: [
        { id: 'label', path: '/label', title: 'Display label', kind: 'text', required: true, default: 'Saved label' },
      ],
    },
  }
  const settings = (id) => ({
    plugin_id: id,
    contract,
    defaults: { label: 'Saved label' },
    configured: { label: 'Saved label' },
    effective: { label: 'Saved label' },
    diagnostics: [],
  })
  try {
    await fs.mkdir(screenshotDir, { recursive: true })
    const entry = path.join(temporary, 'entry.ts')
    const navigation = path.join(temporary, 'navigation.ts')
    await fs.writeFile(
      navigation,
      `
import { useRouter } from 'vue-router'
export function useWorkspaceNavigation() {
  const router = useRouter()
  return { openWorkspaceLocation: async (tab, target) => {
    window.__pluginCardsProbe.launches.push({tab, ...target})
    await router.push({path: target.path, query: target.query, hash: target.hash})
  } }
}
`,
    )
    await fs.writeFile(
      entry,
      `
import { createApp, h, computed, reactive } from 'vue'
import { createPinia } from 'pinia'
import { createRouter, createWebHistory, RouterView, useRoute } from 'vue-router'
import PluginsPanel from '/@fs/${web}/src/components/settings/PluginsPanel.vue'
import AppConfirmHost from '/@fs/${web}/src/components/AppConfirmHost.vue'
import { pluginTabForSettingsView } from '/@fs/${web}/src/components/settings/pluginSettingsNavigation.ts'
import { pluginSettingsLocation } from '/@fs/${web}/src/features/pluginWeb/settingsEntry.ts'
import { useChatStore } from '/@fs/${web}/src/stores/chat.ts'
import { i18n } from '/@fs/${web}/src/i18n/index.ts'
import { settingsText } from '/@fs/${web}/src/i18n/settingsText.ts'
import '/@fs/${web}/src/style.css'

i18n.global.locale.value = 'en-US'
const pinia = createPinia()
useChatStore(pinia).selectedSessionId = '42'
const Panel = { setup() {
  const route = useRoute()
  const tab = computed(() => pluginTabForSettingsView('plugin-workbench', route.query.pluginTab))
  return () => h(PluginsPanel, {tab: tab.value})
} }
const Application = { setup() {
  const route = useRoute()
  return () => h('section', {'data-fixture-application': ''}, [
    h('h1', route.params.pluginId + ' / ' + route.params.pageId),
    h('button', {'data-fixture-return': '', onClick: () => router.push(pluginSettingsLocation())}, 'Return to plugins'),
  ])
} }
const router = createRouter({history: createWebHistory(), routes: [
  {path: '/settings/:section', component: Panel},
  {path: '/plugins/:pluginId/:pageId', component: Application},
]})
const launches = reactive([])
window.__pluginCardsProbe = {
  launches,
  route: () => ({path: router.currentRoute.value.path, query: {...router.currentRoute.value.query}}),
  navigate: (target) => router.push(target),
}
const app = createApp({render: () => h('main', {'data-probe-shell': ''}, [
  h(RouterView), h(AppConfirmHost),
])})
app.config.globalProperties.$st = settingsText
app.use(pinia).use(router).use(i18n)
await router.isReady()
app.mount('#app')
`,
    )
    server = await createServer({
      root: web,
      configFile: path.join(web, 'vite.config.ts'),
      cacheDir: path.join(temporary, 'vite-cache'),
      logLevel: 'warn',
      resolve: { alias: [{ find: '@/app/navigation/useWorkspaceNavigation', replacement: navigation }] },
      server: { host: '127.0.0.1', port: 0, fs: { allow: [path.resolve(web, '../..'), temporary] } },
      plugins: [
        {
          name: 'plugin-cards-fixture',
          configureServer(dev) {
            dev.middlewares.use(async (req, res, next) => {
              const url = new URL(req.url, 'http://fixture')
              if (/^\/(api|auth|health|plugin-assets)(\/|$)/.test(url.pathname)) {
                report.requests.push({ method: req.method, path: url.pathname })
                let body
                if (req.method === 'GET' && url.pathname === '/api/v1/plugins') body = { items: statuses }
                else if (req.method === 'GET' && url.pathname === '/api/v1/plugins/surface')
                  body = { catalog: { commands: [], web_pages: pages } }
                else if (req.method === 'GET' && url.pathname === '/api/v1/plugins/architecture')
                  body = { plugins: [], dependencies: [] }
                else if (req.method === 'GET' && url.pathname === '/api/v1/plugins/fixture.missing') {
                  res.statusCode = 404
                  body = { error: 'Plugin not found' }
                } else {
                  const match = /^\/api\/v1\/plugins\/([^/]+)(\/settings|\/logs)?$/.exec(url.pathname)
                  const status = match && statuses.find((item) => item.plugin_id === decodeURIComponent(match[1]))
                  if (req.method === 'GET' && status) {
                    if (match[2] === '/settings') body = settings(status.plugin_id)
                    else if (match[2] === '/logs') body = { plugin_id: status.plugin_id, logs: [] }
                    else
                      body = {
                        plugin: {
                          status,
                          manifest: {
                            namespace: status.plugin_id,
                            name: status.plugin_id,
                            version: '1.0.0',
                            summary: status.summary,
                            authors: ['Fixture'],
                            settings: contract,
                            tools: [],
                            hooks: [],
                          },
                          hooks: [],
                        },
                      }
                  }
                }
                if (body === undefined) {
                  report.unexpectedApiRequests.push({ method: req.method, path: url.pathname })
                  res.statusCode = 404
                  body = { error: 'Unknown isolated plugin fixture request' }
                }
                res.setHeader('Content-Type', 'application/json')
                return res.end(JSON.stringify(body))
              }
              if (!/^\/(settings|plugins)(\/|$)/.test(url.pathname)) return next()
              const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Plugin cards probe</title></head><body><div id="app"></div><style>html,body,#app{min-height:100%;margin:0}main[data-probe-shell]{height:100dvh;padding:24px;box-sizing:border-box;max-width:1280px;margin:auto}@media(max-width:1023px){html,body,#app{height:auto!important;overflow:visible!important}main[data-probe-shell]{height:auto;padding:16px;min-height:100dvh}}</style><script type="module" src="/@fs/${entry}"></script></body></html>`
              try {
                res.setHeader('Content-Type', 'text/html')
                res.end(await dev.transformIndexHtml(url.pathname, html))
              } catch (error) {
                next(error)
              }
            })
          },
        },
      ],
    })
    await server.listen()
    const origin = 'http://127.0.0.1:' + server.httpServer.address().port
    browser = await chromium.launch({
      executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
      headless: true,
      args: ['--no-sandbox', '--disable-dev-shm-usage'],
    })
    const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, serviceWorkers: 'block' })
    page = await context.newPage()
    page.on('pageerror', (error) => report.browserErrors.push(error.message))
    await page.route('**/*', (route) => {
      if (new URL(route.request().url()).origin === origin) return route.continue()
      report.externalRequests.push(route.request().url())
      return route.abort()
    })
    await page.goto(origin + '/settings/plugins-tools?view=plugin-workbench')
    await page.locator('[data-plugin-card]').first().waitFor({ timeout: 60000 })
    check(
      (await page.locator('[data-plugin-card]').count()) === statuses.length,
      'all installed plugins appear as cards on the initial route',
    )
    check(
      (await page.locator('[data-plugin-grid] aside, [data-plugin-workbench] aside').count()) === 0,
      'plugin content has no internal sidebar',
    )
    check(
      (await page.locator('[data-plugin-detail]').count()) === 0,
      'the initial catalog does not open a plugin detail automatically',
    )
    check(
      await page.evaluate(() => !window.__pluginCardsProbe.route().query.plugin),
      'the initial URL stays on the catalog',
    )
    check(
      !report.requests.some((request) => /^\/api\/v1\/plugins\/fixture\./.test(request.path)),
      'listing cards does not fetch plugin inspect or settings records',
    )
    check((await page.locator('[data-plugin-card="agena.telegram"] svg').count()) > 0, 'cards display plugin icons')
    check(
      (await page.locator('[data-plugin-card="fixture.board"]').innerText()).includes('project dashboard'),
      'cards display plugin descriptions',
    )
    await page.screenshot({ path: path.join(screenshotDir, 'desktop.png'), fullPage: true })

    const search = page.getByRole('textbox', { name: 'Search plugins', exact: true })
    await search.fill('project dashboard')
    await page.waitForFunction(() => document.querySelectorAll('[data-plugin-card]').length === 1)
    check((await page.locator('[data-plugin-card="fixture.board"]').count()) === 1, 'search finds plugin descriptions')
    await search.fill('no-such-plugin')
    await page.getByText('No matching plugins.', { exact: true }).waitFor()
    check((await page.locator('[data-plugin-card]').count()) === 0, 'empty search results have an explicit empty state')
    await page.getByRole('button', { name: 'Clear filters', exact: true }).click()
    await page.waitForFunction(() => document.querySelectorAll('[data-plugin-card]').length === 4)
    await page.getByRole('button', { name: '1 failed', exact: true }).click()
    await page.waitForFunction(() => document.querySelectorAll('[data-plugin-card]').length === 1)
    check((await page.locator('[data-plugin-detail]').count()) === 0, 'failure filtering stays in the card catalog')
    await page.getByRole('button', { name: /State filter:/ }).click()
    await page.locator('[data-option-menu-item]').filter({ hasText: 'All states' }).click()
    await page.waitForFunction(() => document.querySelectorAll('[data-plugin-card]').length === 4)

    const board = '[data-plugin-card="fixture.board"]'
    await page.locator(board + ' [data-plugin-open-settings]').click()
    await page.locator('[data-plugin-detail] input[type="text"]').first().waitFor()
    check(
      await page.evaluate(() => window.__pluginCardsProbe.route().query.pluginTab === 'settings'),
      'card settings action directly selects the settings tab',
    )
    check(
      (await page.locator('[data-plugin-detail] [role="tab"][aria-selected="true"]').innerText()).includes('Settings'),
      'the settings tab matches the route',
    )
    await page.goBack()
    await page.locator('[data-plugin-grid]').waitFor()
    await page.goForward()
    await page.locator('[data-plugin-detail] input[type="text"]').first().waitFor()
    check(
      await page.evaluate(() => window.__pluginCardsProbe.route().query.plugin === 'fixture.board'),
      'browser back and forward restore catalog and plugin settings',
    )
    const input = page.locator('[data-plugin-detail] input[type="text"]').first()
    await input.fill('Unsaved fixture label')
    await page.locator('[data-plugin-back]').click()
    const dialog = page.getByRole('dialog')
    await dialog.waitFor()
    await dialog.getByRole('button', { name: 'Keep editing', exact: true }).click()
    check(
      (await input.inputValue()) === 'Unsaved fixture label',
      'canceling return preserves the unsaved settings draft',
    )
    await page.locator('[data-plugin-back]').click()
    await dialog.getByRole('button', { name: 'Discard changes', exact: true }).click()
    await page.locator('[data-plugin-grid]').waitFor()
    check(
      await page.evaluate(
        () => !window.__pluginCardsProbe.route().query.plugin && !window.__pluginCardsProbe.route().query.pluginTab,
      ),
      'return to cards clears plugin-specific route state',
    )
    await page.locator(board + ' [data-plugin-open-settings]').click()
    await page.locator('[data-plugin-detail] input[type="text"]').first().waitFor()
    check(
      (await page.locator('[data-plugin-detail] input[type="text"]').first().inputValue()) === 'Saved label',
      'discarded edits do not survive opening settings again',
    )
    await page.locator('[data-plugin-back]').click()
    await page.locator(board + ' [data-plugin-open-details]').click()
    await page.locator('[data-plugin-detail] [role="tab"][aria-selected="true"]').waitFor()
    check(
      (await page.locator('[data-plugin-detail] [role="tab"][aria-selected="true"]').innerText()).includes('Overview'),
      'card details opens the full-width overview',
    )
    await page.locator('[data-plugin-back]').click()

    check(
      await page.locator('[data-plugin-card="fixture.stopped"] [data-plugin-open-page]').isDisabled(),
      'stopped plugins keep interaction launch disabled',
    )
    await page.locator(board + ' [data-plugin-app="dashboard"]').click()
    await page.locator('[data-fixture-application]').waitFor()
    check(
      await page.evaluate(() => window.__pluginCardsProbe.route().path === '/plugins/fixture.board/dashboard'),
      'card interaction action navigates to the plugin application',
    )
    await page.locator('[data-fixture-return]').click()
    await page.locator(board + ' [data-plugin-app="history"]').click()
    await page.locator('[data-fixture-application]').waitFor()
    check(
      await page.evaluate(() => !window.__pluginCardsProbe.route().query.sessionId),
      'session-scoped applications do not silently inherit the active chat',
    )
    await page.locator('[data-fixture-return]').click()
    await page.locator('[data-plugin-card="agena.telegram"] [data-plugin-open-page]').click()
    await page.locator('[data-fixture-application]').waitFor()
    check(
      await page.evaluate(() => window.__pluginCardsProbe.route().path === '/plugins/agena.telegram/manage'),
      'Telegram keeps its native interaction launch action',
    )
    await page.locator('[data-fixture-return]').click()

    await page.setViewportSize({ width: 390, height: 844 })
    await page.locator('[data-plugin-grid]').waitFor()
    check(
      await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),
      'mobile plugin cards fit the viewport without horizontal overflow',
    )
    await page.screenshot({ path: path.join(screenshotDir, 'mobile.png'), fullPage: true })
    await page.locator(board + ' [data-plugin-open-settings]').click()
    await page.locator('[data-plugin-detail] input[type="text"]').first().waitFor()
    check(
      await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),
      'mobile plugin settings fit the viewport without horizontal overflow',
    )
    await page.locator('[data-plugin-back]').click()
    await page.locator('[data-plugin-grid]').waitFor()
    check(
      (await page.locator('[data-plugin-card]').count()) === 4,
      'mobile users return to the same complete card catalog',
    )

    await page.evaluate(() =>
      window.__pluginCardsProbe.navigate({
        path: '/settings/plugins-tools',
        query: { view: 'plugin-workbench', plugin: 'fixture.missing', pluginTab: 'settings' },
      }),
    )
    await page.getByText('Plugin details are unavailable.', { exact: true }).waitFor()
    check(
      await page.evaluate(() => window.__pluginCardsProbe.route().query.plugin === 'fixture.missing'),
      'an unknown plugin deep link remains explicit instead of selecting another plugin',
    )
    await page.locator('[data-plugin-back]').click()
    await page.locator('[data-plugin-grid]').waitFor()
    check(
      (await page.locator('[data-plugin-card]').count()) === 4,
      'an unavailable plugin still has a working return to cards',
    )

    check(report.externalRequests.length === 0, 'browser probe made no external requests')
    check(report.unexpectedApiRequests.length === 0, 'all application requests stayed inside known fixtures')
    check(report.browserErrors.length === 0, 'actual Vue components completed without browser exceptions')
    report.passed = true
    console.log('Plugin card browser probe passed: ' + report.checks.length + ' checks')
  } catch (error) {
    report.error = error.stack || String(error)
    if (page) {
      report.dom = await page
        .locator('body')
        .innerText()
        .catch(() => '')
      await page.screenshot({ path: path.join(screenshotDir, 'failure.png'), fullPage: true }).catch(() => {})
    }
    throw error
  } finally {
    await fs.writeFile(reportPath, JSON.stringify(report, null, 2))
    await browser?.close()
    await server?.close()
    await fs.rm(temporary, { recursive: true, force: true })
    console.log('report: ' + reportPath)
    console.log('screenshots: ' + screenshotDir)
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
