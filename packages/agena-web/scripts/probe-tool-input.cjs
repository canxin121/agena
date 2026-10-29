// Real tool cards against an isolated fixture API; never contact a running Agena server.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright')

async function main() {
  const { createServer } = await import('vite')
  const web = path.resolve(__dirname, '..')
  const repo = path.resolve(web, '../..')
  const temporary = await fs.mkdtemp(path.join(web, '.tool-input-probe-'))
  const cases = ['object', 'string', 'lazy'].map((name, index) => ({
    name,
    id: 41 + index,
    input: {
      command: `printf '%s\\n' '${name} input'\npwd`,
      workdir: '/fixture',
      description: `${name} input fixture`,
      reads: [],
      writes: [],
      network: [],
    },
  }))
  let historyState = 'running'
  const inputRequests = []
  const unexpectedApiRequests = []
  const externalRequests = []
  const errors = []
  let server, browser

  function structured(value) {
    if (Array.isArray(value)) return { kind: 'array', items: value.map(structured) }
    if (typeof value === 'string') return { kind: 'text', value }
    return { kind: 'object', fields: Object.entries(value).map(([name, value]) => ({ name, value: structured(value) })) }
  }
  function wireInput(item) {
    return item.name === 'object' ? item.input : JSON.stringify(item.input)
  }
  function historyParts() {
    return cases.map((item) => ({
      part_id: item.id,
      run_id: 11,
      kind: 'tool_call',
      role: 'assistant',
      state: historyState,
      revision: historyState === 'running' ? 1 : 2,
      updated_at_ms: historyState === 'running' ? 1000 : 2000,
      created_at_ms: 1000,
      content: {
        name: 'shell.exec',
        call_id: item.id,
        state: historyState,
        ...(item.name === 'lazy' ? {} : { input: wireInput(item) }),
        tool_api_call: {
          function: 'tools_call',
          arguments: { fields: structured({ tool: 'shell.exec', input: wireInput(item) }).fields },
        },
      },
      sections: [],
      presentation: { title: `${item.name} tool input`, summary: '', blocks: [] },
    }))
  }

  try {
    const entry = path.join(temporary, 'entry.ts')
    await fs.writeFile(entry, `
import { createApp, computed, h, shallowRef } from 'vue'
import { createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import AgenaOperationPart from '/@fs/${web}/src/components/chat/AgenaOperationPart.vue'
import { normalizeAgenaPart } from '/@fs/${web}/src/stores/chat/api.ts'
import { projectLocalPart } from '/@fs/${web}/src/pages/chat/transcriptProjection.ts'
import { i18n } from '/@fs/${web}/src/i18n/index.ts'
import '/@fs/${web}/src/style.css'
const snapshots = shallowRef(await fetch('/__tool-input-fixtures').then(response => response.json()))
const names = ${JSON.stringify(Object.fromEntries(cases.map((item) => [item.id, item.name])))}
const parts = computed(() => snapshots.value.map(raw => {
  const normalized = normalizeAgenaPart(String(raw.part_id), '7', '11', raw)
  if (!normalized) throw new Error('Fixture tool part was lost during normalization')
  return projectLocalPart(normalized, 'assistant')
}))
window.__toolInputProbe = {
  complete: () => { snapshots.value = snapshots.value.map(raw => ({...raw, state:'completed', revision:2, updated_at_ms:2000, content:{...raw.content,state:'completed'}})) },
}
const app = createApp({ setup: () => () => h('main', parts.value.map(part =>
  h('section', {key:part.id, 'data-tool-case':names[part.id], 'data-probe-state':part.status}, [
    h(AgenaOperationPart, {part, expanded:true, collapseSignal:0, sessionId:'7'})
  ])
)) })
app.use(createPinia()).use(createRouter({history:createMemoryHistory(),routes:[{path:'/',component:{render:()=>null}}]})).use(i18n).mount('#app')
`)
    server = await createServer({
      root: web,
      configFile: path.join(web, 'vite.config.ts'),
      logLevel: 'warn',
      server: { host: '127.0.0.1', port: 0, fs: { allow: [repo, temporary] } },
      plugins: [{
        name: 'tool-input-probe-isolation',
        configureServer(dev) {
          dev.middlewares.use(async (req, res, next) => {
            const url = new URL(req.url, 'http://fixture')
            const json = (value) => {
              res.setHeader('Content-Type', 'application/json')
              res.setHeader('Cache-Control', 'no-store')
              res.end(JSON.stringify(value))
            }
            if (url.pathname === '/__tool-input-fixtures' && req.method === 'GET') return json(historyParts())
            if (/^\/(api|auth|health)(\/|$)/.test(url.pathname)) {
              const token = `tool-input-fixture:${historyState === 'running' ? 1000 : 2000}`
              if (url.pathname === '/api/v1/changes/revisions' && req.method === 'GET') {
                let resources
                try {
                  resources = JSON.parse(url.searchParams.get('resources') || '[]')
                } catch {
                  resources = null
                }
                const knownKeys = new Set(cases.map((item) => `part:${item.id}:input`))
                if (Array.isArray(resources) && resources.every((key) => knownKeys.has(key))) {
                  return json(Object.fromEntries(resources.map((key) => [key, token])))
                }
              }
              const match = /^\/api\/v1\/sessions\/7\/parts\/(\d+)\/tool-sections\/input$/.exec(url.pathname)
              const item = match && cases.find((item) => item.id === Number(match[1]))
              if (item && req.method === 'GET') {
                inputRequests.push({ id: item.id, state: historyState })
                res.setHeader('x-resource-revision', token)
                res.setHeader('ETag', `"${token}"`)
                return json({
                  part_id: item.id,
                  revision: historyState === 'running' ? 1 : 2,
                  updated_at_ms: historyState === 'running' ? 1000 : 2000,
                  part_state: historyState,
                  section: 'input',
                  value: wireInput(item),
                })
              }
              unexpectedApiRequests.push({ method: req.method, path: url.pathname })
              res.statusCode = 404
              return json({ error: 'Not available in the isolated tool-input fixture' })
            }
            if (url.pathname !== '/__tool-input-probe') return next()
            const html = `<!doctype html><html><head><meta charset="utf-8"><title>Tool input regression</title></head><body><div id="app"></div><style>html,body,#app{height:auto!important;overflow:visible!important}main{max-width:900px;margin:32px auto}main>section{margin-bottom:24px}</style><script type="module" src="/@fs/${entry}"></script></body></html>`
            try {
              res.setHeader('Content-Type', 'text/html')
              res.end(await dev.transformIndexHtml('/__tool-input-probe', html))
            } catch (error) {
              next(error)
            }
          })
        },
      }],
    })
    await server.listen()
    const origin = `http://127.0.0.1:${server.httpServer.address().port}`
    browser = await chromium.launch({ executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome', headless: true })
    const context = await browser.newContext({ viewport: { width: 1100, height: 1000 }, serviceWorkers: 'block' })
    const page = await context.newPage()
    page.on('pageerror', (error) => errors.push(error.message))
    await page.route('**/*', (route) => {
      if (new URL(route.request().url()).origin === origin) return route.continue()
      externalRequests.push(route.request().url())
      return route.abort()
    })

    async function assertCommand(item, detail = false) {
      const selector = `[data-tool-case="${item.name}"] ${detail
        ? '[data-tool-details] section:has([data-tool-detail-section="input"])'
        : '[data-tool-presentation]'}`
      await page.locator(selector).waitFor({ timeout: 60000 })
      const expected = item.input.command.endsWith('\n') ? item.input.command : `${item.input.command}\n`
      try {
        // Wait for rendered code, not the temporary raw-Markdown fallback.
        await page.waitForFunction(({ selector, expected }) => {
          const element = document.querySelector(selector)
          return element && [...element.querySelectorAll('pre code')].some(code => code.textContent === expected)
        }, { selector, expected })
      } catch (error) {
        error.message += `\n${item.name}, detail=${detail}, visible=${JSON.stringify(await page.locator(selector).innerText())}`
        throw error
      }
      const visible = await page.locator(selector).innerText()
      const diagnostic = `${item.name}, detail=${detail}, visible=${JSON.stringify(visible)}`
      const rendered = await page.locator(selector).locator('pre code').allTextContents()
      assert(rendered.includes(expected), `${diagnostic}: rendered command differs from the original input`)
      if (detail) assert(visible.includes(item.input.workdir), `${diagnostic}: missing input workdir`)
    }
    async function openInput(item) {
      const row = page.locator(`[data-tool-case="${item.name}"]`)
      await row.locator('[data-tool-details-toggle]').click()
      const response = page.waitForResponse((response) =>
        new URL(response.url()).pathname === `/api/v1/sessions/7/parts/${item.id}/tool-sections/input`)
      await row.locator('[data-tool-detail-section="input"]').click()
      assert.equal((await response).status(), 200, `${item.name}: input fixture response failed`)
      await assertCommand(item, true)
      await assertCommand(item)
    }
    async function assertInitialSnapshots() {
      await page.locator('[data-tool-case="lazy"]').waitFor({ timeout: 60000 })
      for (const item of cases.slice(0, 2)) await assertCommand(item)
      const lazy = cases.find((item) => item.name === 'lazy')
      const visible = await page.locator('[data-tool-case="lazy"]').innerText()
      for (const value of [...lazy.input.command.split('\n'), lazy.input.workdir, lazy.input.description]) {
        assert(!visible.includes(value), `omitted input must stay lazy instead of exposing ${value}`)
      }
    }

    await page.goto(origin + '/__tool-input-probe', { waitUntil: 'domcontentloaded' })
    await assertInitialSnapshots()
    assert.equal(inputRequests.length, 0, 'collapsed detail sections must not fetch input')
    for (const item of cases) await openInput(item)
    assert(inputRequests.some((request) => request.id === 43), 'lazy string input must use the detail API')

    historyState = 'completed'
    await page.evaluate(() => window.__toolInputProbe.complete())
    await page.waitForFunction(() => {
      const rows = [...document.querySelectorAll('[data-tool-case]')]
      return rows.length === 3 && rows.every(row => row.dataset.probeState === 'completed')
    })
    for (const item of cases) {
      await assertCommand(item)
      await assertCommand(item, true)
    }

    const beforeReload = inputRequests.length
    await page.reload({ waitUntil: 'domcontentloaded' })
    await assertInitialSnapshots()
    assert.equal(inputRequests.length, beforeReload, 'reloaded history must keep unopened input details lazy')
    for (const item of cases) await openInput(item)
    assert(inputRequests.length >= beforeReload + 3, 'reloaded history must resolve fresh input details')
    assert.deepEqual(unexpectedApiRequests, [], 'all control reads must use the fixture API')
    assert.deepEqual(externalRequests, [], 'the probe must not contact external or live user services')
    assert.deepEqual(errors, [], 'real tool cards must render without browser exceptions')
    console.log('Tool input browser probe passed: object/string snapshots, lazy string Input, running→completed, history reload.')
  } finally {
    await Promise.allSettled([browser?.close(), server?.close()])
    await fs.rm(temporary, { recursive: true, force: true })
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
