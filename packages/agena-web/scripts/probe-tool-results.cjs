// Actual tool cards and every canonical ViewBlock, against an isolated fixture
// API. The browser may not contact a running Agena server or external service.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright')

async function main() {
  const { createServer } = await import('vite')
  const web = path.resolve(__dirname, '..')
  const repo = path.resolve(web, '../..')
  const temporary = await fs.mkdtemp(path.join(web, '.tool-results-probe-'))
  const token = 'tool-results-fixture-token'
  const png = Buffer.from(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4/5/hPwAH/QL+ppTFtAAAAABJRU5ErkJggg==',
    'base64',
  )
  const svg = Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32"><rect width="64" height="32" fill="#2563eb"/></svg>')
  const failures = [
    ['agena.provider', 'The operation failed. Review the logs for details.'],
    ['shell.exec', "$: Additional properties are not allowed ('cwd', 'timeout_seconds' were unexpected)"],
    ['fs.read', '$: "file_path" is a required property'],
    ['fs.read', 'read target does not exist: artifacts/provider-tools/result.json'],
    ['fs.read_media', "$: Additional properties are not allowed ('file_path' was unexpected)"],
    ['fs.read_media', 'fs.read_media requires an active conversation model route'],
    ['chatgpt.image_understanding', '$: "inputs" is a required property'],
    ['chatgpt.image_understanding', 'cloud media analysis returned HTTP 400 Bad Request; inspect status before retrying'],
  ].map(([name, message], index) => ({ name, message, id: 1400 + index }))
  const images = [
    { id: 1146, name: 'generated-image', filename: 'ink-wash-landscape.png', mime: 'image/png', bytes: png },
    { id: 1148, name: 'generated-plot', filename: 'chart.svg', mime: 'image/svg+xml', bytes: svg },
  ].map((image) => ({ ...image, path: `/home/canxin/agena/ai-art/${image.filename}` }))
  const failureParts = failures.map((item) => ({
    part_id: item.id,
    run_id: 11,
    kind: 'tool_call',
    role: 'assistant',
    state: 'failed',
    revision: 5,
    updated_at_ms: 1000,
    created_at_ms: 1000,
    content: {
      name: item.name,
      call_id: item.id,
      state: 'failed',
      error: {
        failure: {
          id: `00000000-0000-4000-8000-${String(item.id).padStart(12, '0')}`,
          code: 'fixture.tool_failure',
          category: 'invalid_input',
          responsibility: 'caller',
          retry: 'correct_input',
          recovery: 'none',
          impact: 'request_rejected',
          user: { key: 'fixture.tool_failure', fallback: item.message },
          model: { text: 'PRIVATE_MODEL_SENTINEL' },
          diagnostic: 'PRIVATE_DIAGNOSTIC_SENTINEL',
        },
      },
    },
    sections: [],
    // Common presentation can repeat the public error; the client error band
    // owns that exact text and must not repeat it in the ordinary body.
    presentation: { title: `${item.name} · failed`, summary: '', blocks: [{ type: 'text', text: item.message }] },
  }))
  function attachment(image) {
    return {
      filename: image.filename,
      kind: 'image',
      mime: image.mime,
      size_bytes: image.bytes.length,
      source: { source: 'local_path', path: image.path },
    }
  }
  const imageParts = images.map((item) => ({
    part_id: item.id,
    run_id: 11,
    kind: 'tool_call',
    role: 'assistant',
    state: 'completed',
    revision: 5,
    updated_at_ms: 1000,
    created_at_ms: 1000,
    content: {
      name: 'chatgpt.image_generate',
      call_id: item.id,
      state: 'completed',
      output: { payload: null, attachments: [attachment(item)], truncated: false },
    },
    sections: [],
    presentation: {
      title: item.name,
      summary: '',
      blocks: [
        { type: 'markdown', id: 'provider-meta', text: 'The generated result.' },
        { type: 'media', id: 'media', artifact: { uri: `file://${item.path}`, name: item.filename, mime: item.mime } },
      ],
    },
  }))
  const blocks = [
    { type: 'text', id: 'text', text: 'Plain text result' },
    { type: 'markdown', id: 'markdown', text: '**Markdown result**\n\n```mermaid\nflowchart TD\nA[Input] --> B[Output]\n```' },
    { type: 'json', id: 'json', value: { json_marker: 17 } },
    { type: 'table', id: 'table', columns: ['name', 'value'], rows: [['table marker', 23]] },
    { type: 'log', id: 'log', stream: 'stderr', text: 'Readable stderr result' },
    { type: 'command', id: 'command', command: 'printf fixture', cwd: '/fixture', stdout: 'Command stdout', stderr: 'Command stderr', exit_code: 1 },
    { type: 'file_changes', id: 'file_changes', changes: [{ path: 'fixture-added.txt', kind: 'added', additions: 2, deletions: 0 }] },
    { type: 'diff', id: 'diff', diff: 'diff --git a/fixture.txt b/fixture.txt\n--- a/fixture.txt\n+++ b/fixture.txt\n@@ -1 +1 @@\n-old line\n+new line\n' },
    { type: 'search_results', id: 'search_results', items: [{ title: 'Search result marker', url: 'https://example.test/result', snippet: 'Search result snippet' }], total: 1 },
    { type: 'media', id: 'media', artifact: { uri: `file://${images[1].path}`, mime: 'image/svg+xml', name: 'chart.svg' } },
    { type: 'content', id: 'content', resource: { resource_id: 'fixture-document', kind: 'document' }, format: 'plain' },
    { type: 'progress', id: 'progress', phase: 'Progress marker', completed: 3, total: 4, unit: 'files' },
    { type: 'custom', id: 'custom', kind: 'schema-only', schema: { schema_marker: 'Custom schema result', points: [1, 2, 3] }, presentation: {} },
  ]
  const documentCursor = { epoch: 'fixture-document', sequence: 1 }
  const documentPage = {
    resource: {
      resource_id: 'fixture-document', kind: 'document', owner_session_id: 48, part_id: 1200, state: 'complete',
      cursor: documentCursor, committed_cursor: documentCursor, retained_ranges: [{ first: 0, last: 1 }],
      total_bytes: null, dropped_bytes: 0, dropped_packets: 0, committed_dropped_bytes: 0, committed_dropped_packets: 0,
    },
    chunks: [{ cursor: documentCursor, captured_at_ms: 1000, payload: {
      type: 'structured_snapshot', document: { blocks: [{ type: 'text', id: 'content-text', text: 'Durable content result' }] },
    } }],
    next_cursor: documentCursor, has_more: false, gap: false,
  }
  const requests = []
  const unexpectedApiRequests = []
  const externalRequests = []
  const errors = []
  let server, browser

  try {
    const entry = path.join(temporary, 'entry.ts')
    await fs.writeFile(entry, `
import { createApp, h, reactive } from 'vue'
import { createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import AgenaOperationPart from '/@fs/${web}/src/components/chat/AgenaOperationPart.vue'
import AgenaOperationBlock from '/@fs/${web}/src/components/chat/AgenaOperationBlock.vue'
import { normalizeAgenaPart } from '/@fs/${web}/src/stores/chat/api.ts'
import { projectLocalPart } from '/@fs/${web}/src/pages/chat/transcriptProjection.ts'
import { writeServerAuthTokenForBaseUrl } from '/@fs/${web}/src/lib/serverAuthToken.ts'
import { i18n } from '/@fs/${web}/src/i18n/index.ts'
import '/@fs/${web}/src/style.css'
i18n.global.locale.value = 'en-US'
writeServerAuthTokenForBaseUrl(location.origin, ${JSON.stringify(token)})
const fixture = await fetch('/__tool-results-fixtures').then(response => response.json())
const parts = fixture.parts.map(raw => {
  const normalized = normalizeAgenaPart(String(raw.part_id), '48', '11', raw)
  if (!normalized) throw new Error('Fixture part was lost during normalization')
  return projectLocalPart(normalized, 'assistant')
})
const expanded = reactive(Object.fromEntries(parts.map(part => [part.id, part.defaultExpanded])))
const app = createApp({ setup: () => () => h('main', [
  ...parts.map(part => h('section', {'data-tool-case':part.id, key:part.id}, [
    h(AgenaOperationPart, {part, expanded:expanded[part.id], collapseSignal:0, sessionId:'48', onToggle:()=>{expanded[part.id]=!expanded[part.id]}})
  ])),
  ...fixture.blocks.map(block => h('section', {'data-block-case':block.type, key:block.id}, [
    h('h2', block.type), h(AgenaOperationBlock, {block, sessionId:'48'})
  ])),
]) })
app.use(createPinia()).use(createRouter({history:createMemoryHistory(),routes:[{path:'/',component:{render:()=>null}}]})).use(i18n).mount('#app')
`)
    server = await createServer({
      root: web,
      configFile: path.join(web, 'vite.config.ts'),
      logLevel: 'warn',
      server: { host: '127.0.0.1', port: 0, fs: { allow: [repo, temporary] } },
      plugins: [{
        name: 'tool-results-probe-isolation',
        configureServer(dev) {
          dev.middlewares.use(async (req, res, next) => {
            const url = new URL(req.url, 'http://fixture')
            const json = (value) => {
              res.setHeader('Content-Type', 'application/json')
              res.setHeader('Cache-Control', 'no-store')
              res.end(JSON.stringify(value))
            }
            if (url.pathname === '/__tool-results-fixtures' && req.method === 'GET')
              return json({ parts: [...failureParts, ...imageParts], blocks })
            if (/^\/(api|auth|health)(\/|$)/.test(url.pathname)) {
              const authenticated = req.headers.authorization === `Bearer ${token}`
              requests.push({ method: req.method, path: url.pathname, query: url.search, authenticated })
              if (req.method !== 'GET' || !authenticated) {
                unexpectedApiRequests.push({ method: req.method, path: url.pathname, authenticated })
                res.statusCode = 403
                return json({ problem: { user: { fallback: 'Only authenticated fixture reads are allowed.' } } })
              }
              if (url.pathname === '/api/v1/changes/revisions') {
                let keys
                try { keys = JSON.parse(url.searchParams.get('resources') || '[]') } catch { keys = null }
                const valid = new Set([...failureParts, ...imageParts].map(part => `part:${part.part_id}:output`))
                if (Array.isArray(keys) && keys.every(key => valid.has(key)))
                  return json(Object.fromEntries(keys.map(key => [key, 'tool-results-fixture:5'])))
              }
              const match = /^\/api\/v1\/sessions\/48\/parts\/(\d+)\/tool-sections\/output$/.exec(url.pathname)
              const part = match && [...failureParts, ...imageParts].find(part => part.part_id === Number(match[1]))
              if (part) {
                const item = images.find(item => item.id === part.part_id)
                res.setHeader('x-resource-revision', 'tool-results-fixture:5')
                res.setHeader('ETag', '"tool-results-fixture:5"')
                return json({ part_id: part.part_id, revision: 5, updated_at_ms: 1000, part_state: part.state,
                  section: 'output', value: item ? { payload: { text: `Loaded output for ${item.name}` }, attachments: [attachment(item)], truncated: false } : null })
              }
              if (url.pathname === '/api/v1/sessions/48/media') {
                const item = images.find(item => item.path === url.searchParams.get('path'))
                if (item) {
                  res.setHeader('Content-Type', item.mime)
                  res.setHeader('Content-Length', item.bytes.length)
                  res.setHeader('Cache-Control', 'private, no-store')
                  res.setHeader('X-Content-Type-Options', 'nosniff')
                  res.setHeader('Content-Disposition', `${url.searchParams.get('download') === 'true' ? 'attachment' : 'inline'}; filename="${item.filename}"`)
                  return res.end(item.bytes)
                }
              }
              if (url.pathname === '/api/v1/sessions/48/content/fixture-document/stream') {
                res.setHeader('Content-Type', 'text/event-stream')
                res.setHeader('Cache-Control', 'no-store')
                return res.end(`event: content\ndata: ${JSON.stringify(documentPage)}\n\n`)
              }
              unexpectedApiRequests.push({ method: req.method, path: url.pathname })
              res.statusCode = 404
              return json({ problem: { user: { fallback: 'Not available in the isolated tool-results fixture.' } } })
            }
            if (url.pathname !== '/__tool-results-probe') return next()
            const html = `<!doctype html><html><head><meta charset="utf-8"><title>Tool results regression</title></head><body><div id="app"></div><style>html,body,#app{height:auto!important;overflow:visible!important}main{max-width:900px;margin:32px auto}main>section{margin-bottom:24px}</style><script type="module" src="/@fs/${entry}"></script></body></html>`
            try {
              res.setHeader('Content-Type', 'text/html')
              res.end(await dev.transformIndexHtml('/__tool-results-probe', html))
            } catch (error) { next(error) }
          })
        },
      }],
    })
    await server.listen()
    const origin = `http://127.0.0.1:${server.httpServer.address().port}`
    browser = await chromium.launch({ executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome', headless: true })
    const context = await browser.newContext({ viewport: { width: 1100, height: 1000 }, serviceWorkers: 'block', acceptDownloads: true })
    const page = await context.newPage()
    page.on('pageerror', error => errors.push(error.message))
    await page.route('**/*', route => {
      if (new URL(route.request().url()).origin === origin) return route.continue()
      externalRequests.push(route.request().url())
      return route.abort()
    })
    await page.goto(origin + '/__tool-results-probe', { waitUntil: 'domcontentloaded' })
    await page.locator('[data-tool-case="1146"]').waitFor({ timeout: 60000 })

    async function assertImage(item) {
      const selector = `[data-tool-case="${item.id}"]`
      await page.locator(selector).scrollIntoViewIfNeeded()
      await page.waitForFunction(selector => {
        const images = [...document.querySelectorAll(`${selector} img`)]
        return images.length === 1 && images.every(image => image.complete && image.naturalWidth > 0 && image.src.startsWith('blob:'))
      }, selector)
      assert.equal(await page.locator(selector).locator('[data-attachment-download]').count(), 1)
    }
    async function openOutput(row, id) {
      const response = page.waitForResponse(response => new URL(response.url()).pathname === `/api/v1/sessions/48/parts/${id}/tool-sections/output`)
      await row.locator('[data-tool-detail-section="output"]').click()
      assert.equal((await response).status(), 200)
    }
    for (const item of failures) {
      const row = page.locator(`[data-tool-case="${item.id}"]`)
      await row.locator('[data-transcript-vim-toggle]').click()
      await row.locator('[data-tool-error]').waitFor()
      assert((await row.locator('[data-tool-error]').innerText()).includes(item.message))
      assert.equal(await row.locator('[data-tool-presentation]').count(), 0, 'failure body must not repeat the error band')
      await row.locator('[data-tool-details-toggle]').click()
      const detail = row.locator('[data-tool-error-details]')
      await detail.waitFor()
      await page.waitForFunction(({ id, message }) => {
        const detail = document.querySelector(`[data-tool-case="${id}"] [data-tool-error-details]`)
        const code = detail?.querySelector('pre code') || detail?.querySelector('pre')
        try {
          const problem = JSON.parse(code?.textContent || '')
          return problem.user?.fallback === message && problem.code === 'fixture.tool_failure'
        } catch { return false }
      }, item)
      for (const key of ['category', 'responsibility', 'retry', 'recovery', 'impact'])
        assert((await detail.innerText()).includes(key), `${item.name}: missing public ${key}`)
      await openOutput(row, item.id)
      const publicError = await detail.evaluate(element => {
        const code = element.querySelector('pre code') || element.querySelector('pre')
        return JSON.parse(code.textContent)
      })
      assert.equal(publicError.user.fallback, item.message, 'a null Output must retain the error reason')
      assert(!(await row.innerText()).includes('PRIVATE_'), 'diagnostic/model fields must never be rendered')
    }
    for (const item of images) {
      const row = page.locator(`[data-tool-case="${item.id}"]`)
      assert.equal(await row.locator('[data-transcript-vim-toggle]').getAttribute('aria-expanded'), 'true')
      await assertImage(item)
      await row.locator('[data-tool-details-toggle]').click()
      await openOutput(row, item.id)
      await assertImage(item)
      await row.locator('[data-tool-detail-section="output"]').click()
      // Reopening can validate the independent section ETag and reuse the
      // already loaded body without another Output GET.
      await row.locator('[data-tool-detail-section="output"]').click()
      await assertImage(item)
      await row.locator('[data-transcript-vim-toggle]').click()
      await row.locator('[data-transcript-vim-toggle]').click()
      await assertImage(item)
      const downloadEvent = page.waitForEvent('download')
      await row.locator('[data-attachment-download]').click()
      const download = await downloadEvent
      assert.equal(download.suggestedFilename(), item.filename)
      const destination = path.join(temporary, item.filename)
      await download.saveAs(destination)
      assert.deepEqual(await fs.readFile(destination), item.bytes)
    }
    const expectedText = {
      text: 'Plain text result', markdown: 'Markdown result', json: 'json_marker', table: 'table marker',
      log: 'Readable stderr result', command: 'Command stderr', file_changes: 'fixture-added.txt', diff: 'new line',
      search_results: 'Search result marker', media: 'chart.svg', content: 'Durable content result',
      progress: '75%', custom: 'Custom schema result',
    }
    for (const [kind, expected] of Object.entries(expectedText)) {
      await page.locator(`[data-block-case="${kind}"]`).scrollIntoViewIfNeeded()
      await page.waitForFunction(({ kind, expected }) => document.querySelector(`[data-block-case="${kind}"]`)?.textContent.includes(expected), { kind, expected })
    }
    await page.locator('[data-block-case="markdown"]').scrollIntoViewIfNeeded()
    await page.locator('[data-block-case="markdown"] [data-oc-mermaid-render] svg').waitFor()
    await page.locator('[data-block-case="media"]').scrollIntoViewIfNeeded()
    await page.waitForFunction(() => {
      const image = document.querySelector('[data-block-case="media"] img')
      return image?.complete && image.naturalWidth === 64
    })
    assert.equal(blocks.length, 13, 'the fixture covers every canonical ViewBlock variant')
    assert(requests.some(request => request.path.endsWith('/content/fixture-document/stream')))
    assert(requests.some(request => request.path.endsWith('/media') && request.query.includes('download=true')))
    assert(requests.every(request => request.authenticated && !request.query.includes(token)))
    assert.deepEqual(unexpectedApiRequests, [], 'all control reads must use the fixture API')
    assert.deepEqual(externalRequests, [], 'the probe must not contact external or live user services')
    assert.deepEqual(errors, [], 'real tool cards must render without browser exceptions')
    console.log('Tool results browser probe passed: 8 public failures, null Output, PNG/SVG default expansion/folds/downloads, all 13 canonical views, Mermaid and authenticated content.')
  } finally {
    await Promise.allSettled([browser?.close(), server?.close()])
    await fs.rm(temporary, { recursive: true, force: true })
  }
}

main().catch(error => {
  console.error(error)
  process.exitCode = 1
})
