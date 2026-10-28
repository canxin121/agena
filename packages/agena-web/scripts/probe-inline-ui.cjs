// True body embedding regression: real answer/transcript components + an isolated
// read-only fixture backend. Never connects to the user's running Agena server.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright')

async function main() {
  const { createServer } = await import('vite')
  const web = path.resolve(__dirname, '..'),
    repo = path.resolve(web, '../..')
  const temporary = await fs.mkdtemp(path.join(web, '.inline-ui-probe-'))
  const output = process.env.INLINE_UI_ARTIFACTS || path.join(repo, 'artifacts/inline-ui')
  await fs.mkdir(output, { recursive: true })
  const sample = JSON.parse(await fs.readFile(path.join(repo, 'examples/inline-ui/reply.json'), 'utf8'))
  const one = 'ui_123_0123456789abcdef0123456789abcdef',
    two = 'ui_124_fedcba9876543210fedcba9876543210'
  const refs = {
    [one]: { version: 1, title: '建议的实施顺序', blocks: sample.blocks.slice(0, 2) },
    [two]: { version: 1, title: '理解差异，再作选择', blocks: sample.blocks.slice(2) },
  }
  const body = `这是一条正常的 **Markdown 回答**，富内容出现在正文指定的位置。\n\n[[inline-ui:${one}]]\n\n第二阶段值得特别说明，下面比较两种能力：\n\n[[inline-ui:${two}]]\n\n最后，先解决内容表达，再逐步扩展能力。`
  const epoch = '11111111-2222-3333-4444-555555555555',
    resourceId = '11111111-2222-3333-4444-666666666666'
  const byteLength = Buffer.byteLength(body)
  const split = body.indexOf(one) + 18
  const fragments = [body.slice(0, split), body.slice(split)]
  function contentPage(text, position, complete, sidecar) {
    const cursor = { epoch, sequence: position }
    return {
      inline_ui: sidecar,
      resource: {
        resource_id: resourceId,
        owner_session_id: 42,
        part_id: 456,
        kind: 'text',
        state: complete ? 'complete' : 'active',
        cursor: { epoch, sequence: byteLength },
        committed_cursor: cursor,
        total_bytes: byteLength,
        dropped_bytes: 0,
        dropped_packets: 0,
        committed_dropped_bytes: 0,
        committed_dropped_packets: 0,
        retained_ranges: [{ first: 1, last: byteLength }],
        capture_error: null,
      },
      chunks: [{ cursor, captured_at_ms: 0, payload: { type: 'text', text } }],
      next_cursor: cursor,
      has_more: !complete,
      gap: false,
    }
  }
  const pages = [
    contentPage(fragments[0], Buffer.byteLength(fragments[0]), false, {}),
    contentPage(fragments[1], byteLength, true, refs),
  ]
  const entry = path.join(temporary, 'entry.ts')
  await fs.writeFile(
    entry,
    `
import { createApp, computed, h, ref, shallowRef } from 'vue'
import { createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import AgenaTranscriptPart from '/@fs/${web}/src/components/chat/AgenaTranscriptPart.vue'
import { replyTextForCopy } from '/@fs/${web}/src/features/inlineUi/copy.ts'
import { expandInlineReferences } from '/@fs/${web}/src/features/inlineUi/references.ts'
import { i18n, setAppLocale } from '/@fs/${web}/src/i18n/index.ts'
import '/@fs/${web}/src/style.css'
const body = ref(${JSON.stringify(body)}), documents = shallowRef(${JSON.stringify(refs)}), session = ref('42'), streaming = ref(false), role = ref('assistant'), sourceMode = ref(false)
let revision = 1
const answer = computed(() => {
  const content = sourceMode.value ? {text:'',resources:[{resource_id:${JSON.stringify(resourceId)},kind:'text'}]} : {text:body.value}
  return {id:'456',kind:role.value==='assistant'?'answer':'text',role:role.value,status:streaming.value?'running':'completed',title:'Answer',summary:'',copyText:'',
    source:{id:'456',agenaKind:'text',agenaRole:role.value,partState:streaming.value?'running':'completed',revision,agenaContent:content,agenaInlineUi:documents.value,text:content.text}}
})
const prepared = {id:'123',kind:'operation',role:'assistant',status:'completed',title:'Prepared inline content',summary:'Ready to embed',copyText:'',
  source:{id:'123',agenaKind:'tool_call',partState:'completed',revision:1,tool:'inline_ui.prepare',agenaContent:{name:'inline_ui.prepare',state:'completed'},agenaPresentation:{title:'Prepared inline content',summary:'Ready to embed in the answer',blocks:[{type:'text',id:'prepared',text:'Content is prepared; only the body reference displays it.'}]}}}
window.__inlineProbe = {
  setBody:(text, live=false)=>{revision++;body.value=text;streaming.value=live;sourceMode.value=false},
  setDocuments:(value)=>{documents.value=value}, setSession:(value)=>{session.value=value;documents.value={}},
  setRole:(value)=>{role.value=value}, setSourceMode:(value)=>{sourceMode.value=value;documents.value={}},
  expectedCopy:()=>expandInlineReferences(${JSON.stringify(body)},${JSON.stringify(refs)}),
}
setAppLocale('zh-CN')
const app = createApp({setup:()=>()=>h('main',{class:'probe-page'},[
  h('header',{class:'probe-header'},'AGENA / INLINE GENERATIVE UI'),
  h(AgenaTranscriptPart,{part:prepared,expanded:false,collapseSignal:0,sessionId:session.value}),
  h(AgenaTranscriptPart,{part:answer.value,expanded:true,collapseSignal:0,sessionId:session.value,streaming:streaming.value}),
  h('button',{'data-probe-copy':'',onClick:async()=>{const text=await replyTextForCopy([answer.value.source],(part)=>part.text,session.value,role.value==='assistant');await navigator.clipboard.writeText(text)}},'复制整条回答')
])})
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
      server: { host: '127.0.0.1', port: 0, fs: { allow: [repo, temporary] } },
      plugins: [
        {
          name: 'inline-ui-probe-isolation',
          configureServer(dev) {
            dev.middlewares.use(async (req, res, next) => {
              const url = new URL(req.url, 'http://fixture')
              if (/^\/(api|auth|health)(\/|\?|$)/.test(req.url || '')) {
                apiRequests.push(req.url)
                const match = /^\/api\/v1\/sessions\/(\d+)\/inline-ui\/(.+)$/.exec(url.pathname)
                if (match && match[1] === '42' && refs[match[2]]) {
                  res.setHeader('Content-Type', 'application/json')
                  res.end(JSON.stringify(refs[match[2]]))
                  return
                }
                if (url.pathname === `/api/v1/sessions/42/content/${resourceId}/stream`) {
                  res.setHeader('Content-Type', 'text/event-stream')
                  res.write('event: content\ndata: ' + JSON.stringify(pages[0]) + '\n\n')
                  setTimeout(() => res.end('event: content\ndata: ' + JSON.stringify(pages[1]) + '\n\n'), 60)
                  return
                }
                if (url.pathname === `/api/v1/sessions/42/content/${resourceId}`) {
                  res.setHeader('Content-Type', 'application/json')
                  res.end(JSON.stringify(contentPage(body, byteLength, true, refs)))
                  return
                }
                res.statusCode = 404
                res.setHeader('Content-Type', 'application/json')
                res.end(JSON.stringify({ error: 'Unavailable in this conversation' }))
                return
              }
              if (req.url !== '/__inline-ui-probe') return next()
              const html = `<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Inline body embedding</title></head><body><div id="app"></div><style>html,body,#app{height:auto!important;min-height:100%;overflow:visible!important}body{margin:0;background:var(--color-background);color:var(--color-foreground)}.probe-page{width:calc(100% - 40px);max-width:860px;margin:40px auto;padding-bottom:40px}.probe-header{font-size:11px;letter-spacing:.15em;opacity:.55;margin-bottom:28px}button{cursor:pointer}[data-probe-copy]{font-size:12px;margin-top:24px;color:var(--color-muted-foreground)}</style><script type="module" src="/@fs/${entry}"></script></body></html>`
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
    await page.locator('[data-inline-embed]').nth(1).waitFor({ timeout: 60000 })
    assert.equal(await page.locator('[data-inline-reply]').count(), 2, 'only body references display rich content')
    assert.equal(await page.locator('[data-inline-kind]').count(), 8)
    assert.equal(
      await page.locator('[data-inline-reply-part]').count(),
      0,
      'preparation is not a rich tool-result card',
    )
    assert.equal(await page.locator('iframe').count(), 0)
    const visible = await page.locator('main').innerText()
    assert(visible.indexOf('这是一条正常的') < visible.indexOf('建议的实施顺序'))
    assert(visible.indexOf('第二阶段值得特别说明') < visible.indexOf('理解差异，再作选择'))
    assert(visible.indexOf('理解差异，再作选择') < visible.indexOf('最后，先解决'))
    assert(!visible.includes('[[inline-ui:'))
    await page.locator('[data-probe-copy]').click()
    assert.equal(
      await page.evaluate(() => navigator.clipboard.readText()),
      await page.evaluate(() => window.__inlineProbe.expectedCopy()),
    )
    await page.locator('.reply-details summary').focus()
    await page.keyboard.press('Enter')
    assert.equal(await page.locator('.reply-details').getAttribute('open'), '')
    await page.evaluate((body) => window.__inlineProbe.setBody(body + '\n\n正在继续解释。', true), body)
    assert.equal(
      await page.locator('.reply-details').getAttribute('open'),
      '',
      'appending prose must not remount an existing embed',
    )
    await page.evaluate((body) => window.__inlineProbe.setBody(body), body)
    await page.screenshot({ path: path.join(output, 'body-desktop-light.png'), fullPage: true })
    await page.evaluate(() => document.documentElement.classList.add('dark'))
    await page.screenshot({ path: path.join(output, 'body-desktop-dark.png'), fullPage: true })
    await page.setViewportSize({ width: 375, height: 812 })
    await page.screenshot({ path: path.join(output, 'body-mobile-dark.png'), fullPage: true })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
    assert.deepEqual(apiRequests, [], 'prepared data sidecars replay without reads/writes')
    // Partial streaming markers must not appear as raw protocol text.
    await page.evaluate(() => window.__inlineProbe.setBody('前文\n\n[[inline-ui:ui_123_', true))
    await page.waitForFunction(() => !!document.querySelector('[data-inline-reference-pending]'))
    assert(!(await page.locator('main').innerText()).includes('[[inline-ui:'))
    await page.evaluate((body) => window.__inlineProbe.setBody(body), body)
    await page.locator('[data-inline-embed]').nth(1).waitFor()
    // Code and user-authored examples stay literal, not live embeds.
    await page.evaluate(
      (marker) => window.__inlineProbe.setBody('```text\n' + marker + '\n```'),
      `[[inline-ui:${one}]]`,
    )
    await page.waitForFunction(() => document.querySelectorAll('[data-inline-embed]').length === 0)
    await page.evaluate((marker) => {
      window.__inlineProbe.setRole('user')
      window.__inlineProbe.setBody(marker)
    }, `[[inline-ui:${one}]]`)
    assert.equal(await page.locator('[data-inline-embed]').count(), 0)
    await page.evaluate((body) => {
      window.__inlineProbe.setRole('assistant')
      window.__inlineProbe.setBody(body)
      window.__inlineProbe.setDocuments({})
    }, body)
    await page.locator('[data-inline-reply]').nth(1).waitFor()
    assert(
      apiRequests.filter((url) => url.includes('/inline-ui/')).length >= 2,
      'history resolves without cached tool messages',
    )
    // Actual content-resource/SSE path, with the reference split across pages.
    await page.evaluate(() => window.__inlineProbe.setSourceMode(true))
    await page.waitForFunction(() => document.querySelectorAll('[data-inline-reply]').length === 2)
    await page.locator('[data-probe-copy]').click()
    assert.equal(
      await page.evaluate(() => navigator.clipboard.readText()),
      await page.evaluate(() => window.__inlineProbe.expectedCopy()),
    )
    // Changing sessions must not retain resolved content from the previous session.
    await page.evaluate((body) => {
      window.__inlineProbe.setSourceMode(false)
      window.__inlineProbe.setBody(body)
      window.__inlineProbe.setSession('43')
    }, body)
    await page.waitForFunction(
      () =>
        document.querySelectorAll('[data-inline-reply]').length === 0 &&
        document.querySelectorAll('[data-inline-embed] [role="status"]').length === 2,
    )
    assert(apiRequests.some((url) => url.startsWith('/api/v1/sessions/43/inline-ui/')))
    assert.deepEqual(
      requests.filter((r) => !['GET', 'HEAD'].includes(r.method)),
      [],
      'rendering never writes',
    )
    assert.deepEqual(errors, [])
    const checks = [
      'normal final-answer body placement',
      'two ordered embeds with prose between',
      'preparation never renders duplicate UI',
      '8 kinds',
      'whole-answer copy expands references',
      'keyboard details',
      'append-only streaming preserves disclosure',
      'light/dark/mobile',
      'incomplete marker hidden',
      'code/user examples inert',
      'history without tool cache',
      'SSE split-reference delivery',
      'session switch clears content',
      'no write requests',
    ]
    await fs.writeFile(
      path.join(output, 'body-results.json'),
      JSON.stringify(
        {
          passed: true,
          checks,
          screenshots: ['body-desktop-light.png', 'body-desktop-dark.png', 'body-mobile-dark.png'],
        },
        null,
        2,
      ),
    )
    console.log(`Inline body embedding browser probe passed. Artifacts: ${output}`)
  } finally {
    if (browser) await browser.close()
    if (server) await server.close()
    await fs.rm(temporary, { recursive: true, force: true })
  }
}
main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
