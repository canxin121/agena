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
    iconRequests: [],
    layouts: {},
  }
  let browser, server, page
  const check = (condition, description) => {
    assert(condition, description)
    report.checks.push(description)
    console.log('PASS ' + description)
  }
  const checkCardLayout = async (viewport) => {
    const layout = await page.locator('[data-plugin-card]').evaluateAll((cards) => {
      const bounds = (element) => {
        if (!element) return null
        const { x, y, width, height } = element.getBoundingClientRect()
        return { x, y, width, height }
      }
      const longCard = cards.find((card) => card.dataset.pluginCard.includes('very-long-plugin-identifier'))
      const title = longCard.querySelector('h2')
      const summary = longCard.querySelector(':scope > p')
      const titleStyle = getComputedStyle(title)
      const summaryStyle = getComputedStyle(summary)
      return {
        cards: cards.map((card) => ({
          id: card.dataset.pluginCard,
          ...bounds(card),
          interaction: bounds(card.querySelector('[data-plugin-interaction]')),
          settings: bounds(card.querySelector('[data-plugin-open-settings]')),
        })),
        titleEllipsis:
          titleStyle.textOverflow === 'ellipsis' &&
          titleStyle.whiteSpace === 'nowrap' &&
          title.scrollWidth > title.clientWidth,
        summaryClamped:
          summaryStyle.webkitLineClamp === '3' &&
          summaryStyle.overflow === 'hidden' &&
          summary.scrollHeight > summary.clientHeight,
      }
    })
    report.layouts[viewport] = layout
    const first = layout.cards[0]
    check(
      layout.cards.every((card) => Math.abs(card.width - first.width) < 1 && Math.abs(card.height - first.height) < 1),
      viewport + ' cards have identical width and height across rows',
    )
    if (viewport === 'desktop') {
      check(
        new Set(layout.cards.map((card) => Math.round(card.y))).size > 1,
        'desktop uniform size check spans multiple card rows',
      )
    }
    const interactions = layout.cards.filter((card) => card.interaction)
    check(
      interactions.length === 3 &&
        interactions.every(
          ({ interaction, settings }) =>
            interaction.x + interaction.width <= settings.x + 1 &&
            Math.abs(interaction.y + interaction.height / 2 - settings.y - settings.height / 2) < 1,
        ),
      viewport + ' interaction buttons sit to the left of settings on the same row',
    )
    check(
      layout.titleEllipsis && layout.summaryClamped,
      viewport + ' long plugin titles and descriptions are truncated without growing cards',
    )
  }
  const iconSvg =
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" rx="5" fill="#268bd2"/><path d="m6 12 4 4 8-8" stroke="white" stroke-width="2" fill="none"/></svg>'
  const statuses = [
    {
      plugin_id: 'agena.telegram',
      display_name: 'Telegram',
      icon: 'data:image/svg+xml;base64,' + Buffer.from(iconSvg).toString('base64'),
      authors: ['canxin121'],
      version: '0.1.0',
      tags: ['channels', 'messaging'],
      kind: 'builtin',
      state: 'running',
      restart_count: 0,
      summary: 'Connect conversations and manage Telegram integrations.',
    },
    {
      plugin_id: 'fixture.board',
      display_name: 'Project board',
      authors: ['canxin121'],
      version: '1.2.3',
      tags: ['projects', 'workspace'],
      kind: 'stdio',
      state: 'running',
      restart_count: 0,
      summary: 'A project dashboard with history and interactive reports.',
    },
    {
      plugin_id: 'fixture.stopped',
      icon: '/fixture-icons/missing.svg',
      authors: ['canxin121'],
      version: '0.1.0',
      kind: 'stdio',
      state: 'stopped',
      restart_count: 0,
      summary: 'An installed application that is currently stopped.',
    },
    {
      plugin_id: 'fixture.very-long-plugin-identifier-for-responsive-layout-and-keyboard-navigation',
      icon: 'javascript:window.__unsafeIcon=true',
      authors: ['canxin121'],
      version: '0.1.0',
      kind: 'builtin',
      state: 'failed',
      restart_count: 0,
      summary:
        'A long description with enough information to verify ellipsis on small screens and readable card actions. Additional details explain the plugin features, supported integrations, configuration options, history views, and workspace activity. This intentionally extended description remains longer than three lines on desktop and mobile so every card keeps the same fixed height while its actions stay aligned.',
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
import AppDialogHost from '/@fs/${web}/src/components/AppDialogHost.vue'
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
  h(RouterView), h(AppDialogHost),
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
              if (url.pathname.startsWith('/fixture-icons/')) {
                report.iconRequests.push({ path: url.pathname, referer: req.headers.referer || null })
                res.setHeader('Content-Type', 'image/svg+xml')
                if (url.pathname === '/fixture-icons/board.svg') return res.end(iconSvg)
                res.statusCode = 404
                return res.end()
              }
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
                            display_name: status.display_name,
                            icon: status.icon,
                            version: status.version || '1.0.0',
                            summary: status.summary,
                            authors: status.authors || [],
                            tags: status.tags || [],
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
    statuses[1].icon = origin + '/fixture-icons/board.svg'
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
    await page.waitForFunction(() => {
      const icon = document.querySelector('[data-plugin-card="agena.telegram"] img[data-plugin-icon]')
      return icon?.complete && icon.naturalWidth > 0
    })
    check(
      (await page.locator('[data-plugin-card="agena.telegram"] img[data-plugin-icon]').getAttribute('src')).startsWith(
        'data:image/svg+xml;base64,',
      ),
      'cards display base64-encoded icons declared by the plugin',
    )
    await page.waitForFunction(() => {
      const icon = document.querySelector('[data-plugin-card="fixture.board"] img[data-plugin-icon]')
      return icon?.complete && icon.naturalWidth > 0
    })
    check(
      (await page.locator('[data-plugin-card="fixture.board"] img[data-plugin-icon]').getAttribute('src')) ===
        statuses[1].icon,
      'cards load plugin-declared absolute HTTP image URLs',
    )
    check(
      report.iconRequests.every((request) => request.referer === null),
      'plugin icon requests omit the page referrer',
    )
    await page.locator('[data-plugin-card="fixture.stopped"] [data-plugin-icon-fallback]').waitFor()
    check(
      (await page.locator('[data-plugin-card="fixture.stopped"] img[data-plugin-icon]').count()) === 0,
      'an image load failure shows the generic plugin icon',
    )
    check(
      (await page.locator('[data-plugin-card*="very-long-plugin-identifier"] [data-plugin-icon-fallback]').count()) ===
        1,
      'unsafe icon sources fall back before an image request is made',
    )
    const encodedSvg =
      'data:image/svg+xml;charset=utf-8,' +
      encodeURIComponent(iconSvg.replace('</svg>', '<script>window.__unsafeIcon=true</script></svg>'))
    statuses[2].icon = encodedSvg
    await page.getByRole('button', { name: 'Refresh plugins', exact: true }).click()
    await page.waitForFunction(() => {
      const icon = document.querySelector('[data-plugin-card="fixture.stopped"] img[data-plugin-icon]')
      return icon?.complete && icon.naturalWidth > 0
    })
    check(
      (await page.locator('[data-plugin-card="fixture.stopped"] img[data-plugin-icon]').getAttribute('src')) ===
        encodedSvg,
      'changing icon metadata retries a failed image and accepts percent-encoded SVG',
    )
    check(
      await page.evaluate(() => window.__unsafeIcon === undefined),
      'SVG image metadata cannot execute as page markup',
    )
    check(
      (await page.locator('[data-plugin-card="fixture.board"] h2').innerText()) === 'Project board' &&
        (await page.locator('[data-plugin-card="fixture.board"] [data-plugin-card-metadata]').innerText()) ===
          'canxin121 · v1.2.3',
      'cards show plugin-declared names, authors and versions',
    )
    check(
      (await page.locator('[data-plugin-card="fixture.board"]').innerText()).includes('project dashboard'),
      'cards display plugin descriptions',
    )
    await checkCardLayout('desktop')
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
    const detailTabs = page.getByRole('tablist', { name: 'Plugin details' })
    const settingsTab = detailTabs.getByRole('tab', { name: /^Settings/ })
    const toolsTab = detailTabs.getByRole('tab', { name: /^Tools/ })
    const selectedTab = page.locator('[aria-selected="true"][tabindex="0"]')
    await input.fill('Unsaved fixture label')
    await settingsTab.press('ArrowRight')
    await toolsTab.and(selectedTab).waitFor()
    check(
      (await toolsTab.innerText()).match(/\d+/) &&
        (await toolsTab.evaluate((tab) => tab === document.activeElement)) &&
        (await page.evaluate(() => window.__pluginCardsProbe.route().query.pluginTab === 'tools')),
      'arrow navigation focuses the tab with its count and updates the plugin route',
    )
    await settingsTab.getByTitle('You have unsaved changes.').waitFor()
    await toolsTab.press('ArrowLeft')
    await settingsTab.and(selectedTab).waitFor()
    check((await input.inputValue()) === 'Unsaved fixture label', 'switching tabs retains the plugin draft')
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
    await page.locator('[data-plugin-detail] dl').first().waitFor()
    const detailText = await page.locator('[data-plugin-detail]').innerText()
    check(
      ['Project board', 'fixture.board', 'canxin121', '1.2.3', 'projects', 'workspace'].every((value) =>
        detailText.includes(value),
      ),
      'plugin details expose the complete declared metadata',
    )
    await page.locator('[data-plugin-back]').click()

    check(
      await page.locator('[data-plugin-card="fixture.stopped"] [data-plugin-open-page]').isDisabled(),
      'stopped plugins keep interaction launch disabled',
    )
    await page.locator(board + ' [data-plugin-interaction]').focus()
    await page.locator(board + ' [data-plugin-interaction]').press('Enter')
    await page.locator('[data-option-menu-item]').first().waitFor()
    check(
      (await page.locator('[data-option-menu-item]').count()) === 2,
      'multiple interaction pages share one card action and remain available in its menu',
    )
    check(
      await page.locator('[data-option-menu-item]').evaluateAll((items) =>
        items.every((item) => {
          const rect = item.getBoundingClientRect()
          return (
            rect.width > 0 &&
            rect.height > 0 &&
            rect.top >= 0 &&
            rect.bottom <= innerHeight &&
            item.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2))
          )
        }),
      ),
      'interaction menu options remain visible and clickable outside the clipped card',
    )
    await page.screenshot({ path: path.join(screenshotDir, 'desktop-menu.png'), fullPage: true })
    await page.locator('[data-option-menu-item]').filter({ hasText: 'Dashboard' }).focus()
    await page.locator('[data-option-menu-item]').filter({ hasText: 'Dashboard' }).press('Enter')
    await page.locator('[data-fixture-application]').waitFor()
    check(
      await page.evaluate(() => window.__pluginCardsProbe.route().path === '/plugins/fixture.board/dashboard'),
      'keyboard interaction action and menu selection navigate to the plugin application',
    )
    await page.locator('[data-fixture-return]').click()
    await page.locator(board + ' [data-plugin-interaction]').click()
    await page.locator('[data-option-menu-item]').filter({ hasText: 'History' }).click()
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
    await checkCardLayout('mobile')
    await page.screenshot({ path: path.join(screenshotDir, 'mobile.png'), fullPage: true })
    await page.locator(board + ' [data-plugin-open-settings]').click()
    await page.locator('[data-plugin-detail] input[type="text"]').first().waitFor()
    check(
      await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),
      'mobile plugin settings fit the viewport without horizontal overflow',
    )
    const diagnosticsTab = detailTabs.getByRole('tab', { name: /^Diagnostics/ })
    const overviewTab = detailTabs.getByRole('tab', { name: 'Overview', exact: true })
    await settingsTab.press('End')
    await diagnosticsTab.and(selectedTab).waitFor()
    check(
      await detailTabs.evaluate((list) => {
        const tabs = [...list.querySelectorAll('[role="tab"]')]
        const panels = tabs.map((tab) => document.getElementById(tab.getAttribute('aria-controls')))
        const selected = tabs.find((tab) => tab === document.activeElement)
        const bounds = list.getBoundingClientRect()
        return list.scrollWidth > list.clientWidth && selected && selected.getBoundingClientRect().right <= bounds.right + 1 &&
          selected.matches(':focus-visible') && getComputedStyle(selected).boxShadow !== 'none' &&
          tabs.filter((tab) => tab.tabIndex === 0).length === 1 && new Set(panels).size === tabs.length &&
          panels.every((panel, index) => panel?.getAttribute('role') === 'tabpanel' &&
            panel.getAttribute('aria-labelledby') === tabs[index].id &&
            (panel.getClientRects().length > 0) === (tabs[index] === selected))
      }),
      'End scrolls the overflowing mobile tabs to their visible focus ring and each tab owns a panel',
    )
    await diagnosticsTab.press('Home')
    await overviewTab.and(selectedTab).waitFor()
    await detailTabs.evaluate((list) => { list.dir = 'rtl' })
    await overviewTab.press('ArrowLeft')
    await settingsTab.and(selectedTab).waitFor()
    check(
      await settingsTab.evaluate((tab) => tab === document.activeElement),
      'the physical Left key advances to the next route-backed tab in RTL',
    )
    await detailTabs.evaluate((list) => list.removeAttribute('dir'))
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
