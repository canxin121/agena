// Isolated browser regression probe for the current public interaction contract.
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
  const temporary = await fs.mkdtemp(path.join(web, '.baseline-interaction-probe-'))
  const reportPath =
    process.env.PROBE_REPORT || path.join(os.tmpdir(), 'agena-new-baseline-interaction-browser-report.json')
  const report = { passed: false, checks: [], browserErrors: [], externalRequests: [], unexpectedApiRequests: [] }
  let browser, server, page
  const check = (condition, description) => {
    assert(condition, description)
    report.checks.push(description)
  }
  console.log('stage: prepare current interaction fixtures')
  try {
    const entry = path.join(temporary, 'entry.ts')
    await fs.writeFile(
      entry,
      `
import { createApp, h, reactive, computed } from 'vue'
import { createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import AgenaInteractionPart from '/@fs/${web}/src/components/chat/AgenaInteractionPart.vue'
import { interactionPresentationFromAttention } from '/@fs/${web}/src/pages/chat/transcriptPartPresentation.ts'
import { useChatStore } from '/@fs/${web}/src/stores/chat.ts'
import { pluginTabForSettingsView } from '/@fs/${web}/src/components/settings/pluginSettingsNavigation.ts'
import { settingsQueryForDestination } from '/@fs/${web}/src/components/settings/settingsDestination.ts'
import { i18n } from '/@fs/${web}/src/i18n/index.ts'
import '/@fs/${web}/src/style.css'

const pinia = createPinia()
const chat = useChatStore(pinia)
const drafts = new Map()
const pending = reactive(new Set(['literal', 'explicit', 'cancel']))
const replies = reactive([])
const request = (id, question, inputKind = 'ask_user') => interactionPresentationFromAttention({
  kind: 'question', payload: {properties: {id, request: {
    request_id: id, kind: 'user_input', input_kind: inputKind, title: id, questions: [question],
  }}},
})
const plainQuestion = {header: 'Choice', question: 'Pick a fixture answer', allow_custom: true, options: [
  {label: 'Custom', description: 'Ordinary named choice'},
  {label: 'Type your own answer', description: 'Another ordinary named choice'},
  {label: 'Custom_', description: 'Another current literal choice'},
]}
const cases = reactive({
  literal: request('literal', plainQuestion),
  explicit: request('explicit', {header: 'Feedback', question: 'Provide feedback', allow_custom: false, options: [{label: 'Feedback', description: 'Explicit custom editor', custom: true}]}),
  cancel: request('cancel', {header: 'Cancel', question: 'Cancel this request', allow_custom: false, options: [{label: 'Continue', description: ''}]}),
  history: {...request('history', plainQuestion), pending: false, reply: {request_id: 'history', kind: 'submit', cancelled: true, answers: {'0': ['Custom']}}},
  revoked: request('revoked', plainQuestion),
})
chat.getPendingInteraction = (sessionId, requestId, kind) => pending.has(requestId) ? {sessionId, requestId, kind, request: {request_id: requestId}} : null
chat.hasInteractionSnapshot = () => true
chat.getInteractionDraft = (_sessionId, requestId) => {
  if (!drafts.has(requestId)) drafts.set(requestId, reactive({selected: [], customDrafts: [], customOpen: [], questionPage: 0, busy: false}))
  return drafts.get(requestId)
}
chat.replyQuestion = async (sessionId, requestId, answers) => {
  if (sessionId !== '7' || !pending.has(requestId)) return false
  replies.push({kind: 'submit', sessionId, requestId, answers})
  const reply = {request_id: requestId, kind: 'submit', answers: Object.fromEntries(answers.map((answer, index) => [String(index), answer]))}
  cases[requestId].reply = reply
  cases[requestId].pending = false
  pending.delete(requestId)
  return true
}
chat.rejectQuestion = async (sessionId, requestId) => {
  if (sessionId !== '7' || !pending.has(requestId)) return false
  replies.push({kind: 'cancel', sessionId, requestId})
  cases[requestId].reply = {request_id: requestId, kind: 'cancel'}
  cases[requestId].pending = false
  pending.delete(requestId)
  return true
}
const router = createRouter({history: createMemoryHistory(), routes: [{path: '/settings/:section', component: {render: () => null}}]})
await router.push({path: '/settings/plugins-tools', query: {view: 'plugin-workbench', plugin: 'fixture', pluginTab: 'settings'}})
window.__interactionProbe = {
  replies,
  route: () => ({...router.currentRoute.value.query}),
  navigate: async (destination) => router.push({path: '/settings/' + destination.section, query: settingsQueryForDestination(destination, router.currentRoute.value.params.section, router.currentRoute.value.query)}),
}
const app = createApp({setup: () => {
  const tab = computed(() => pluginTabForSettingsView(router.currentRoute.value.query.view, router.currentRoute.value.query.pluginTab))
  return () => h('main', [
    h('output', {'data-current-plugin-tab': ''}, tab.value || 'none'),
    ...Object.entries(cases).map(([id, interaction]) => h('section', {'data-case': id, key: id}, [h(AgenaInteractionPart, {interaction, sessionId: '7'})])),
  ])
}})
app.use(pinia).use(router).use(i18n).mount('#app')
`,
    )
    server = await createServer({
      root: web,
      configFile: path.join(web, 'vite.config.ts'),
      logLevel: 'warn',
      server: { host: '127.0.0.1', port: 0, fs: { allow: [path.resolve(web, '../..'), temporary] } },
      plugins: [
        {
          name: 'current-interaction-fixture',
          configureServer(dev) {
            dev.middlewares.use(async (req, res, next) => {
              const url = new URL(req.url, 'http://fixture')
              if (/^\/(api|auth|health)(\/|$)/.test(url.pathname)) {
                report.unexpectedApiRequests.push({ method: req.method, path: url.pathname })
                res.statusCode = 404
                res.setHeader('Content-Type', 'application/json')
                return res.end(JSON.stringify({ error: 'No API is available in the isolated interaction probe' }))
              }
              if (url.pathname !== '/__current-interaction-probe') return next()
              const html = `<!doctype html><html><head><meta charset="utf-8"><title>Current interaction contracts</title></head><body><div id="app"></div><style>html,body,#app{height:auto!important;overflow:visible!important}main{max-width:850px;margin:20px auto}main>section{padding:12px;margin:12px 0;border:1px solid #888}</style><script type="module" src="/@fs/${entry}"></script></body></html>`
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
    const origin = `http://127.0.0.1:${server.httpServer.address().port}`
    console.log('stage: launch isolated browser')
    browser = await chromium.launch({
      executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
      headless: true,
      args: ['--no-sandbox', '--disable-dev-shm-usage'],
    })
    const context = await browser.newContext({ viewport: { width: 1100, height: 950 }, serviceWorkers: 'block' })
    page = await context.newPage()
    page.on('pageerror', (error) => report.browserErrors.push(error.message))
    await page.route('**/*', (route) => {
      if (new URL(route.request().url()).origin === origin) return route.continue()
      report.externalRequests.push(route.request().url())
      return route.abort()
    })
    await page.goto(origin + '/__current-interaction-probe')
    await page.locator('[data-case="literal"] [data-interaction-options]').waitFor({ timeout: 60000 })
    console.log('stage: current literal and explicit custom options')
    const literal = page.locator('[data-case="literal"]')
    check(
      (await literal.locator('[data-interaction-control-kind="option"]').count()) === 3,
      'literal Custom, Type your own answer and Custom_ remain ordinary choices',
    )
    check(
      (await literal.locator('[data-interaction-control-kind="custom"]').count()) === 1,
      'allow_custom adds exactly one distinct editor',
    )
    const values = await literal
      .locator('input')
      .evaluateAll((inputs) => inputs.map((input) => input.getAttribute('data-interaction-control-value')))
    check(new Set(values).size === values.length, 'synthetic custom identity does not collide with producer labels')
    await literal.locator('[data-interaction-control-kind="option"][data-interaction-control-value="Custom"]').check()
    check(
      (await literal.locator('textarea').count()) === 0,
      'selecting the literal Custom value does not open a text editor',
    )
    await literal.locator('[data-interaction-control-key="page:next"]').click()
    await literal.locator('[data-interaction-result]').waitFor()
    check(
      await literal
        .locator('[data-interaction-result]')
        .innerText()
        .then((text) => text.includes('Custom')),
      'submitted literal value remains readable in committed history',
    )
    check(
      (await page.evaluate(() => JSON.stringify(window.__interactionProbe.replies[0]?.answers))) === '[["Custom"]]',
      'literal selection submits the original label',
    )

    const explicit = page.locator('[data-case="explicit"]')
    check(
      (await explicit.locator('[data-interaction-control-kind="custom"]').count()) === 1,
      'explicit custom option works when allow_custom is false',
    )
    await explicit.locator('[data-interaction-control-kind="custom"]').check()
    const editor = explicit.locator('textarea')
    await editor.waitFor()
    await page.waitForFunction(
      () => document.querySelector('[data-case="explicit"] textarea') === document.activeElement,
    )
    check(
      await editor.evaluate((element) => element === document.activeElement),
      'custom selection transfers focus into its editor',
    )
    await editor.fill('Fixture freeform response')
    await explicit.locator('[data-interaction-control-key="page:next"]').click()
    await explicit.locator('[data-interaction-result]').waitFor()
    check(
      (await explicit.innerText()).includes('Fixture freeform response'),
      'freeform response survives current reply projection',
    )
    check(
      (await page.evaluate(() => JSON.stringify(window.__interactionProbe.replies[1]?.answers))) ===
        '[["Fixture freeform response"]]',
      'freeform answer is submitted without its UI marker',
    )

    console.log('stage: cancellation and authoritative request ownership')
    const cancelled = page.locator('[data-case="cancel"]')
    await cancelled.locator('[data-interaction-control-key="page:review"]').click()
    await cancelled.locator('[data-interaction-control-key="action:reject"]').click()
    await cancelled.locator('[data-interaction-result]').waitFor()
    check((await cancelled.innerText()).includes('Cancelled'), 'current reply.kind=cancel displays cancellation')
    const history = page.locator('[data-case="history"]')
    check((await history.innerText()).includes('Answered'), 'retired cancelled boolean does not override kind=submit')
    check(
      !(await history.innerText()).includes('Cancelled'),
      'submitted history is not reclassified by an obsolete field',
    )
    const revoked = page.locator('[data-case="revoked"]')
    check(
      (await revoked.locator('input, textarea, [data-interaction-control="true"]').count()) === 0,
      'a request absent from current authoritative ownership has no active reply controls',
    )
    check(
      (await page.evaluate(() => window.__interactionProbe.replies.length)) === 3,
      'only explicitly performed authorized fixture replies are sent',
    )

    console.log('stage: actual router with current settings navigation contracts')
    check(
      (await page.locator('[data-current-plugin-tab]').innerText()) === 'settings',
      'explicit current workbench route selects its plugin tab',
    )
    await page.evaluate(() =>
      window.__interactionProbe.navigate({ section: 'plugins-tools', view: 'plugin-workbench' }),
    )
    check(
      (await page.evaluate(() => window.__interactionProbe.route().pluginTab)) === 'settings',
      'workbench navigation preserves its current tab',
    )
    await page.evaluate(() => window.__interactionProbe.navigate({ section: 'plugins-tools', view: 'marketplace' }))
    await page.waitForFunction(() => document.querySelector('[data-current-plugin-tab]').textContent === 'none')
    check(
      await page.evaluate(
        () => !('plugin' in window.__interactionProbe.route()) && !('pluginTab' in window.__interactionProbe.route()),
      ),
      'leaving workbench clears plugin-specific route state',
    )
    await page.evaluate(() => window.__interactionProbe.navigate({ section: 'plugins-tools', view: 'plugin-settings' }))
    check(
      (await page.locator('[data-current-plugin-tab]').innerText()) === 'none',
      'retired sidebar route does not upgrade itself into a workbench',
    )

    check(report.externalRequests.length === 0, 'probe never contacted external or real user services')
    check(report.unexpectedApiRequests.length === 0, 'isolated interactions did not fall through to an API')
    check(report.browserErrors.length === 0, 'real interaction components rendered without browser exceptions')
    report.passed = true
    console.log(`Current interaction and navigation probe passed: ${report.checks.length} checks`)
  } catch (error) {
    report.error = error.stack || String(error)
    if (page)
      report.dom = await page
        .locator('body')
        .innerText()
        .catch(() => '')
    throw error
  } finally {
    await fs.writeFile(reportPath, JSON.stringify(report, null, 2))
    await browser?.close()
    await server?.close()
    await fs.rm(temporary, { recursive: true, force: true })
    console.log(`report: ${reportPath}`)
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
