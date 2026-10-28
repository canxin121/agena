// Full interactive/live regression against the real transcript renderer and an
// isolated SSE fixture. It never starts a model or contacts a real Agena server.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright')

async function main() {
  const { createServer } = await import('vite')
  const web = path.resolve(__dirname, '..'),
    repo = path.resolve(web, '../..')
  const temp = await fs.mkdtemp(path.join(web, '.inline-ui-probe-'))
  const output = process.env.INLINE_UI_ARTIFACTS || path.join(repo, 'artifacts/inline-ui')
  await fs.mkdir(output, { recursive: true })
  const document = JSON.parse(await fs.readFile(path.join(repo, 'examples/inline-ui/live-dashboard.json'), 'utf8'))
  const reference = 'ui_123_0123456789abcdef0123456789abcdef'
  let snapshot = { reference, revision: 1, phase: 'working', updated_at_ms: 1, last_call_id: 123, document }
  const initial = structuredClone(snapshot)
  const connections = new Set(),
    requests = [],
    errors = []
  let opened = 0
  function emit(value, retain = true) {
    if (retain) snapshot = value
    for (const res of connections)
      res.write(`event: inline_ui\nid: ${value.revision}\ndata: ${JSON.stringify(value)}\n\n`)
  }
  const body =
    '下面是可以探索的交互示例。数据和公式均为模拟。\n\n```inline-ui\n' +
    reference +
    '\n```\n\n界面会随着检查进展更新，你的筛选和输入不会被重置。'
  const entry = path.join(temp, 'entry.ts')
  await fs.writeFile(
    entry,
    `
import {createApp,computed,h,ref} from 'vue'
import {createPinia} from 'pinia'
import {createRouter,createMemoryHistory} from 'vue-router'
import AgenaTranscriptPart from '/@fs/${web}/src/components/chat/AgenaTranscriptPart.vue'
import {replyTextForCopy} from '/@fs/${web}/src/features/inlineUi/copy.ts'
import {setAppLocale,i18n} from '/@fs/${web}/src/i18n/index.ts'
import '/@fs/${web}/src/style.css'
const session=ref('42'),body=ref(${JSON.stringify(body)}),visible=ref(true),duplicate=ref(false)
const part=computed(()=>({id:'456',kind:'answer',role:'assistant',status:'running',title:'Answer',summary:'',copyText:'',source:{id:'456',agenaKind:'text',agenaRole:'assistant',partState:'running',revision:1,agenaContent:{text:body.value},text:body.value}}))
window.__liveProbe={setSession:value=>{session.value=value},setVisible:value=>{visible.value=value},setDuplicate:value=>{duplicate.value=value}}
setAppLocale('zh-CN')
const app=createApp({setup:()=>()=>h('main',{class:'probe-page'},[
 h('header',{class:'probe-header'},'AGENA / LIVE INTERACTIVE REPLY'),
 visible.value?h(AgenaTranscriptPart,{part:part.value,expanded:true,collapseSignal:0,sessionId:session.value}):null,
 duplicate.value?h('section',{'data-duplicate-ui':''},[h(AgenaTranscriptPart,{part:part.value,expanded:true,collapseSignal:0,sessionId:session.value})]):null,
 h('button',{'data-copy-answer':'',onClick:async()=>{const text=await replyTextForCopy([part.value.source],p=>p.text,session.value,true);await navigator.clipboard.writeText(text)}},'复制整条回答')
])})
app.use(createPinia()).use(createRouter({history:createMemoryHistory(),routes:[{path:'/',component:{render:()=>null}}]})).use(i18n).mount('#app')
`,
  )
  let server, browser, pageForFailure
  try {
    server = await createServer({
      root: web,
      configFile: path.join(web, 'vite.config.ts'),
      logLevel: 'warn',
      server: { host: '127.0.0.1', port: 0, fs: { allow: [repo, temp] } },
      plugins: [
        {
          name: 'stateful-ui-fixture',
          configureServer(dev) {
            dev.middlewares.use(async (req, res, next) => {
              const url = new URL(req.url, 'http://fixture')
              if (url.pathname.startsWith('/api/')) {
                if (url.pathname === `/api/v1/sessions/42/inline-ui/${reference}/stream`) {
                  opened++
                  res.setHeader('Content-Type', 'text/event-stream')
                  res.setHeader('Cache-Control', 'no-store')
                  res.write(`event: inline_ui\nid: ${snapshot.revision}\ndata: ${JSON.stringify(snapshot)}\n\n`)
                  connections.add(res)
                  const keepalive = setInterval(() => res.write(': alive\n\n'), 1000)
                  req.on('close', () => {
                    clearInterval(keepalive)
                    connections.delete(res)
                  })
                  return
                }
                if (
                  url.pathname === `/api/v1/sessions/42/inline-ui/${reference}/live` ||
                  url.pathname === `/api/v1/sessions/42/inline-ui/${reference}`
                ) {
                  res.setHeader('Content-Type', 'application/json')
                  res.end(JSON.stringify(url.pathname.endsWith('/live') ? snapshot : snapshot.document))
                  return
                }
                res.statusCode = 404
                res.setHeader('Content-Type', 'application/json')
                res.end(JSON.stringify({ error: 'Not available in this session' }))
                return
              }
              if (req.url !== '/__stateful-ui') return next()
              const html = `<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Live Inline UI</title></head><body><div id="app"></div><style>html,body,#app{height:auto!important;min-height:100%;overflow:visible!important}body{margin:0;background:var(--color-background);color:var(--color-foreground)}.probe-page{width:calc(100% - 40px);max-width:920px;margin:40px auto;padding-bottom:48px}.probe-header{font-size:11px;letter-spacing:.15em;opacity:.55;margin-bottom:24px}[data-copy-answer]{margin-top:24px;font-size:12px;opacity:.7}</style><script type="module" src="/@fs/${entry}"></script></body></html>`
              try {
                res.setHeader('Content-Type', 'text/html')
                res.end(await dev.transformIndexHtml('/__stateful-ui', html))
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
      viewport: { width: 1240, height: 1050 },
      permissions: ['clipboard-read', 'clipboard-write'],
    })
    const page = await context.newPage()
    pageForFailure = page
    page.on('pageerror', (e) => errors.push(e.message))
    page.on('request', (r) => requests.push({ url: r.url(), method: r.method() }))
    await page.route('**/*', (route) => (route.request().url().startsWith(origin) ? route.continue() : route.abort()))
    await page.goto(origin + '/__stateful-ui', { waitUntil: 'domcontentloaded' })
    await page.locator('[data-interactive-panel]').waitFor({ timeout: 60000 })
    await page.waitForFunction(
      () => document.querySelector('[data-inline-revision]')?.getAttribute('data-inline-revision') === '1',
    )
    const metric = (id) => page.locator(`[data-ui-node="${id}"] .ui-metric-value`)
    assert((await metric('revenue').innerText()).includes('118500'))
    await page.getByRole('tab', { name: '项目执行', exact: true }).click()
    await page.locator('[data-ui-node="taskList"] input').nth(1).check()
    await page.getByRole('button', { name: '返回总览查看进度', exact: true }).click()
    assert(
      Math.abs(
        Number(await page.locator('[data-ui-node="taskProgress"] [role="progressbar"]').getAttribute('aria-valuenow')) -
          200 / 3,
      ) < 1e-8,
    )
    await page.getByLabel('渠道', { exact: true }).selectOption('paid')
    assert((await metric('revenue').innerText()).includes('39000'))
    assert.equal(await page.locator('[data-ui-node="salesTable"] tbody tr').count(), 2)
    await page.screenshot({ path: path.join(output, 'live-overview-light.png'), fullPage: true })
    // Keyboard tabs and inputs operate without any model call or backend write.
    await page.getByRole('tab', { name: '经营总览', exact: true }).focus()
    await page.keyboard.press('End')
    await page.locator('[data-ui-node="budget"] input').focus()
    await page.keyboard.press('End')
    assert((await metric('scenarioProfit').innerText()).includes('-9700'))
    assert.equal(await page.locator('[data-ui-node="simWarning"]').count(), 1)
    await page.getByLabel('显示计算假设', { exact: true }).check()
    assert.equal(await page.locator('[data-ui-node="formula"]').count(), 1)
    await page.getByRole('button', { name: '复制当前摘要', exact: true }).click()
    await page.waitForFunction(async () => (await navigator.clipboard.readText()).includes('-9700'))
    const copied = await page.evaluate(() => navigator.clipboard.readText())
    assert(copied.includes('39000') && copied.includes('2/3'))
    await page.locator('[data-ui-node="budget"] input').focus()
    const focused = await page.evaluate(() => document.activeElement.id)
    const next = structuredClone(snapshot)
    next.revision = 2
    next.last_call_id = 124
    next.updated_at_ms = 2
    const program = next.document.blocks[0].program
    program.data.processed = 6
    program.data.workLabel = '检查完成'
    program.data.timeline.push({ id: 'done', title: '检查完成', detail: '已核验全部记录。' })
    program.nodes.find((n) => n.id === 'simTitle').props.text = '情景推演 · 数据已更新'
    emit(next)
    await page.waitForFunction(
      () => document.querySelector('[data-inline-revision]')?.getAttribute('data-inline-revision') === '2',
    )
    assert.equal(await page.locator('[data-ui-node="budget"] input').inputValue(), '60000')
    assert.equal(await page.evaluate(() => document.activeElement.id), focused, 'data updates preserve keyboard focus')
    assert.equal(await page.getByRole('tab', { name: '情景推演', exact: true }).getAttribute('aria-selected'), 'true')
    assert.equal(await page.getByLabel('显示计算假设', { exact: true }).isChecked(), true)
    assert.equal(await page.locator('[data-ui-node="simTitle"]').innerText(), '情景推演 · 数据已更新')
    emit(initial, false)
    emit(next, false)
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
    assert.equal(
      await page.locator('[data-inline-revision]').getAttribute('data-inline-revision'),
      '2',
      'old or duplicate revisions do not roll back UI',
    )
    assert.equal(
      await page.evaluate(() => navigator.clipboard.readText()),
      copied,
      'snapshots never replay clipboard actions',
    )
    await page.evaluate(() => document.documentElement.classList.add('dark'))
    await page.screenshot({ path: path.join(output, 'live-simulation-dark.png'), fullPage: true })
    await page.setViewportSize({ width: 375, height: 850 })
    await page.screenshot({ path: path.join(output, 'live-mobile-dark.png'), fullPage: true })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
    // Reconnect from an actual disconnected SSE stream; the latest full snapshot wins.
    const third = structuredClone(next)
    third.revision = 3
    third.phase = 'complete'
    third.document.blocks[0].program.data.workLabel = '已完成并保存'
    snapshot = third
    const before = opened
    for (const res of [...connections]) res.end()
    await page.waitForFunction(
      () => document.querySelector('[data-inline-revision]')?.getAttribute('data-inline-revision') === '3',
    )
    assert(opened > before)
    assert.equal(await page.locator('[data-ui-node="budget"] input').inputValue(), '60000')
    await page.getByRole('tab', { name: '经营总览', exact: true }).click()
    assert.equal(await page.getByLabel('渠道', { exact: true }).inputValue(), 'paid')
    assert.equal(
      await page.locator('[data-ui-node="agentProgress"] [role="progressbar"]').getAttribute('aria-valuenow'),
      '6',
    )
    assert(
      Math.abs(
        Number(await page.locator('[data-ui-node="taskProgress"] [role="progressbar"]').getAttribute('aria-valuenow')) -
          200 / 3,
      ) < 1e-8,
    )
    await page.locator('[data-copy-answer]').click()
    await page.waitForFunction(async () => {
      const text = await navigator.clipboard.readText()
      return text.includes('39000') && text.includes('已完成并保存')
    })
    // Remount restores client-local state in this browser tab, not agent data.
    await page.evaluate(() => window.__liveProbe.setVisible(false))
    await page.waitForFunction(() => !document.querySelector('[data-interactive-panel]'))
    // Simulate a stale first server frame after remount. Previously known
    // revisions are a floor, even when a new stream reader is created.
    snapshot = initial
    const remounted = page.waitForResponse((response) => response.url().endsWith('/stream'))
    await page.evaluate(() => window.__liveProbe.setVisible(true))
    await remounted
    await page.waitForTimeout(80)
    assert.notEqual(await page.locator('[data-inline-embed]').getAttribute('data-inline-revision'), '1')
    emit(third)
    await page.locator('[data-interactive-panel]').waitFor()
    assert.equal(await page.getByLabel('渠道', { exact: true }).inputValue(), 'paid')
    const reconnect = page.waitForResponse((response) => response.url().endsWith('/stream'))
    const malformed = structuredClone(snapshot)
    malformed.revision = 99
    malformed.document.blocks[0].program.nodes[0].kind = 'execute-javascript'
    emit(malformed, false)
    await reconnect
    await page.waitForFunction(
      () => document.querySelector('[data-inline-revision]')?.getAttribute('data-inline-revision') === '3',
    )
    assert.equal(
      await page.getByLabel('渠道', { exact: true }).inputValue(),
      'paid',
      'invalid snapshots preserve the last good UI and input',
    )
    // Reusing a reference shares input state, not DOM ids or focus targets.
    await page.evaluate(() => window.__liveProbe.setDuplicate(true))
    await page.locator('[data-duplicate-ui]').scrollIntoViewIfNeeded()
    await page.waitForFunction(() => document.querySelectorAll('[data-interactive-panel]').length === 2)
    const ids = await page
      .locator('[data-interactive-panel] input[id], [data-interactive-panel] select[id]')
      .evaluateAll((elements) => elements.map((element) => element.id))
    assert.equal(new Set(ids).size, ids.length, 'repeat embeds require distinct DOM control ids')
    await page.locator('[data-duplicate-ui]').getByLabel('渠道', { exact: true }).selectOption('all')
    await page.waitForFunction(() =>
      [...document.querySelectorAll('[data-ui-node="channel"] select')].every((element) => element.value === 'all'),
    )
    await page.locator('[data-duplicate-ui]').getByLabel('渠道', { exact: true }).selectOption('paid')
    await page.evaluate(() => window.__liveProbe.setDuplicate(false))
    await page.locator('[data-inline-embed]').scrollIntoViewIfNeeded()
    const denied = page.waitForResponse(
      (response) => response.url().includes('/api/v1/sessions/43/inline-ui/') && response.status() === 404,
    )
    await page.evaluate(() => window.__liveProbe.setSession('43'))
    await page.waitForFunction(
      () =>
        !document.querySelector('[data-interactive-panel]') &&
        document.querySelector('[data-inline-embed] [role="status"]'),
    )
    await page.locator('[data-inline-embed]').scrollIntoViewIfNeeded()
    await denied
    await page.waitForFunction(() =>
      document
        .querySelector('[data-inline-embed] [role="status"]')
        ?.textContent.includes('这份引用内容在当前会话中不可用'),
    )
    assert.deepEqual(
      requests.filter((r) => !['GET', 'HEAD'].includes(r.method)),
      [],
      'local input and SSE must never trigger external writes',
    )
    assert.deepEqual(errors, [])
    const checks = [
      'real inline transcript, not tool card',
      '45-node composable UI',
      'task/overview shared state',
      'filter updates metrics, chart and table',
      'keyboard tabs and slider',
      'simulation/conditional formula/warning',
      'clipboard only on click',
      'AI revision updates existing nodes',
      'input/focus survive AI update',
      'stale/duplicate revision ignored',
      'reconnect fetches latest snapshot',
      'current-state whole-answer copy',
      'remount retains tab-local input',
      'remount cannot regress to an old revision',
      'duplicate embeds share state but have unique DOM ids',
      'invalid snapshot keeps last valid UI',
      'cross-session access denied',
      'light/dark/mobile',
      'no backend side effects',
    ]
    await fs.writeFile(
      path.join(output, 'live-results.json'),
      JSON.stringify(
        {
          passed: true,
          checks,
          screenshots: ['live-overview-light.png', 'live-simulation-dark.png', 'live-mobile-dark.png'],
        },
        null,
        2,
      ),
    )
    console.log(`Stateful live UI browser probe passed. Artifacts: ${output}`)
  } catch (error) {
    if (pageForFailure) {
      await pageForFailure.screenshot({ path: path.join(output, 'live-failure.png'), fullPage: true }).catch(() => {})
      console.error('Browser errors:', errors)
      console.error((await pageForFailure.locator('main').innerText()).slice(0, 1500))
    }
    throw error
  } finally {
    for (const res of connections) res.end()
    if (browser) await browser.close()
    if (server) await server.close()
    await fs.rm(temp, { recursive: true, force: true })
  }
}
main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
