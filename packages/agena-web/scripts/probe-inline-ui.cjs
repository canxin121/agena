// Exercises real transcript rendering against an isolated Vite server and inert
// fixtures. Never connects to the user's Agena backend or runs a model/service.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright')

async function main() {
  const { createServer } = await import('vite')
  const web = path.resolve(__dirname, '..')
  const repo = path.resolve(web, '../..')
  const temp = await fs.mkdtemp(path.join(web, '.inline-ui-probe-'))
  const output = process.env.INLINE_UI_ARTIFACTS || path.join(repo, 'artifacts/inline-ui')
  await fs.mkdir(output, { recursive: true })
  const fixture = JSON.parse(await fs.readFile(path.join(repo, 'examples/inline-ui/reply.json'), 'utf8'))
  const expectedCopy = (await fs.readFile(path.join(repo, 'examples/inline-ui/reply.md'), 'utf8')).trimEnd()
  const entry = path.join(temp, 'entry.ts')
  await fs.writeFile(
    entry,
    `
import { createApp, computed, h, shallowRef } from 'vue'
import { createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import AgenaTranscriptPart from '/@fs/${web}/src/components/chat/AgenaTranscriptPart.vue'
import { i18n, setAppLocale } from '/@fs/${web}/src/i18n/index.ts'
import '/@fs/${web}/src/style.css'
const documentValue = shallowRef(${JSON.stringify(fixture)})
let revision = 1
const part = computed(() => ({
  id:'2', kind:'operation', role:'assistant', status:'completed', title:'Inline Generative UI', summary:'', copyText:'',
  source: {id:'2', type:'tool_call', revision, tool:'inline_ui.present', agenaContent:{name:'inline_ui.present',state:'completed'},
    agenaPresentation:{title:'Inline Generative UI',summary:'',blocks:[{type:'inline_ui',id:'inline-reply',document:documentValue.value}]}}
}))
window.__inlineProbe = { setDocument: (value) => { revision++; documentValue.value = value }, setLocale: setAppLocale }
setAppLocale('zh-CN')
const app = createApp({ setup: () => () => h('main', {class:'probe-page'}, [
  h('header', {class:'probe-header'}, 'AGENA / INLINE GENERATIVE UI'),
  h(AgenaTranscriptPart, {part:part.value, expanded:false, collapseSignal:0, sessionId:'42'})
]) })
app.use(createPinia()).use(createRouter({history:createMemoryHistory(),routes:[{path:'/',component:{render:()=>null}}]})).use(i18n).mount('#app')
`,
  )
  let server, browser
  const requests = [],
    errors = [],
    apiRequests = []
  try {
    server = await createServer({
      root: web,
      configFile: path.join(web, 'vite.config.ts'),
      logLevel: 'warn',
      server: { host: '127.0.0.1', port: 0, fs: { allow: [repo, temp] } },
      plugins: [
        {
          name: 'inline-ui-probe-isolation',
          configureServer(dev) {
            dev.middlewares.use(async (req, res, next) => {
              if (/^\/(api|auth|health)(\/|\?|$)/.test(req.url || '')) {
                apiRequests.push(req.url)
                res.statusCode = 404
                res.setHeader('Content-Type', 'application/json')
                res.end(JSON.stringify({ error: 'No backend is available in this isolated probe' }))
                return
              }
              if (req.url !== '/__inline-ui-probe') return next()
              const html = `<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Inline Generative UI probe</title></head><body><div id="app"></div><style>html,body,#app{height:auto!important;min-height:100%;overflow:visible!important}body{margin:0;background:var(--color-background);color:var(--color-foreground)}.probe-page{width:calc(100% - 40px);max-width:860px;margin:40px auto;padding-bottom:40px}.probe-header{font-size:11px;letter-spacing:.15em;opacity:.55;margin-bottom:28px}button{cursor:pointer}</style><script type="module" src="/@fs/${entry}"></script></body></html>`
              try {
                res.setHeader('Content-Type', 'text/html')
                res.end(await dev.transformIndexHtml('/__inline-ui-probe', html))
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
    browser = await chromium.launch({
      executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
      headless: true,
    })
    const context = await browser.newContext({
      viewport: { width: 1180, height: 900 },
      permissions: ['clipboard-read', 'clipboard-write'],
    })
    const page = await context.newPage()
    page.on('pageerror', (error) => errors.push(error.message))
    page.on('request', (request) => requests.push({ url: request.url(), method: request.method() }))
    await page.route('**/*', (route) => (route.request().url().startsWith(origin) ? route.continue() : route.abort()))
    await page.goto(origin + '/__inline-ui-probe', { waitUntil: 'networkidle' })
    await page.locator('[data-inline-reply]').waitFor({ timeout: 60000 })
    assert.equal(await page.locator('[data-inline-kind]').count(), 8)
    assert.equal(await page.locator('.reply-step').count(), 3)
    assert.equal(await page.locator('[data-interactive-app]').count(), 0)
    assert.equal(await page.locator('iframe').count(), 0)
    assert.equal(await page.getByRole('button', { name: 'Run', exact: true }).count(), 0)
    await page.getByRole('button', { name: '复制这段内容', exact: true }).click()
    assert.equal(await page.evaluate(() => navigator.clipboard.readText()), expectedCopy)
    await page.locator('.reply-details summary').focus()
    await page.keyboard.press('Enter')
    assert.equal(await page.locator('.reply-details').getAttribute('open'), '')
    await page.evaluate((value) => window.__inlineProbe.setDocument(value), fixture)
    assert.equal(
      await page.locator('.reply-details').getAttribute('open'),
      '',
      'disclosure should survive same-id content updates',
    )
    await page.screenshot({ path: path.join(output, 'desktop-light.png'), fullPage: true })
    await page.evaluate(() => document.documentElement.classList.add('dark'))
    await page.screenshot({ path: path.join(output, 'desktop-dark.png'), fullPage: true })
    await page.setViewportSize({ width: 375, height: 812 })
    await page.screenshot({ path: path.join(output, 'mobile-dark.png'), fullPage: true })
    assert.equal(
      await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),
      true,
      'document must not overflow on mobile',
    )
    const hostile = structuredClone(fixture)
    hostile.blocks[0].text = '<img src=x onerror="window.__inlineXss=true"> [click](javascript:alert(1))'
    await page.evaluate((value) => window.__inlineProbe.setDocument(value), hostile)
    await page.waitForFunction(() =>
      document.querySelector('[data-inline-kind="paragraph"]').textContent.includes('<img'),
    )
    assert.equal(await page.locator('[data-inline-reply] img').count(), 0)
    assert.equal(await page.evaluate(() => window.__inlineXss), undefined)
    assert.deepEqual(
      requests.filter((request) => !['GET', 'HEAD'].includes(request.method)),
      [],
      'rendering must not perform writes',
    )
    assert.deepEqual(apiRequests, [], 'replay with a supplied projection must not fetch or mutate the backend')
    assert.deepEqual(errors, [], 'browser runtime errors')
    await fs.writeFile(
      path.join(output, 'results.json'),
      JSON.stringify(
        {
          passed: true,
          checks: [
            'real transcript, tools collapsed',
            '8 content kinds',
            'copy matches Rust/TUI',
            'keyboard disclosure',
            'disclosure survives updates',
            'light and dark themes',
            '375px overflow',
            'no iframe or app',
            'inert hostile text',
            'no backend requests or writes',
          ],
          screenshots: ['desktop-light.png', 'desktop-dark.png', 'mobile-dark.png'],
        },
        null,
        2,
      ),
    )
    console.log(`Inline Generative UI browser probe passed. Artifacts: ${output}`)
  } finally {
    if (browser) await browser.close()
    if (server) await server.close()
    await fs.rm(temp, { recursive: true, force: true })
  }
}
main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
