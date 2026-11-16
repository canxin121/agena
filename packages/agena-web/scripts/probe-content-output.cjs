// Real streaming output in Vue and Chromium, using only an isolated fixture API.
// Optional: PLAYWRIGHT_MODULE, CHROME_BIN, PROBE_REPORT, PROBE_SCREENSHOT_DIR,
// PROBE_BASELINE=1, PROBE_OFFLINE=1, PROBE_HIDDEN_MOUNT=1.
// PROBE_BASELINE reads the HEAD component through Vite without changing the checkout.
// PROBE_OFFLINE uses in-process Vite transforms, intercepted requests and an
// in-page SSE transport. Chromium communicates over a pipe; no listener or
// outbound network connection is created.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const { execFileSync } = require('node:child_process')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright')

async function main() {
  const { createServer } = await import('vite')
  const web = path.resolve(__dirname, '..')
  const repo = path.resolve(web, '../..')
  const temporary = await fs.mkdtemp(path.join(web, '.content-output-probe-'))
  const offline = process.env.PROBE_OFFLINE === '1'
  const hiddenMount = process.env.PROBE_HIDDEN_MOUNT === '1'
  const component = path.join(web, 'src/components/chat/AgenaContentOutput.vue')
  const baseline =
    process.env.PROBE_BASELINE === '1'
      ? execFileSync('git', ['show', 'HEAD:packages/agena-web/src/components/chat/AgenaContentOutput.vue'], {
          cwd: repo,
          encoding: 'utf8',
        })
      : null
  const epoch = '11111111-1111-4111-8111-111111111111'
  const token = 'content-output-fixture'
  const resources = new Map(
    [
      ['short'],
      ['long'],
      ['offscreen'],
      ['terminal', 'terminal'],
      ['draining'],
      ['manual-follow'],
      ['interrupted'],
      ['empty-live'],
      ['short-final'],
      ['history-log'],
      ['history-empty'],
      ['history-terminal', 'terminal'],
      ['history-markdown', 'log', 'markdown'],
      ['history-json', 'log', 'json'],
      ['history-code', 'log', 'code'],
      ['history-diff', 'log', 'diff'],
      ['cache-0'],
      ['cache-1'],
      ['cache-2'],
      ['cache-3'],
    ].map(([id, kind = 'log', format = 'plain']) => [
      id,
      { id, kind, format, state: 'active', finalSequence: null, chunks: [], clients: new Set() },
    ]),
  )
  const checks = [],
    geometry = [],
    errors = [],
    unexpected = [],
    external = [],
    requests = []
  let server, browser, page
  const pendingEmits = []
  const heldResources = new Set()
  const check = (value, message) => {
    assert.ok(value, message)
    checks.push(message)
  }
  const stage = (message) => console.error('[content-output-probe] ' + message)
  function pageFor(resource, after = 0) {
    const cursor = { epoch, sequence: resource.finalSequence ?? resource.chunks.length }
    const nextCursor = { epoch, sequence: resource.chunks.length }
    return {
      resource: {
        resource_id: resource.id,
        kind: resource.kind,
        owner_session_id: 48,
        part_id: 1200,
        state: resource.state,
        cursor,
        committed_cursor: cursor,
        retained_ranges: [{ first: 0, last: cursor.sequence }],
        total_bytes: null,
        dropped_bytes: 0,
        dropped_packets: 0,
        committed_dropped_bytes: 0,
        committed_dropped_packets: 0,
      },
      chunks: resource.chunks.filter((chunk) => chunk.cursor.sequence > after),
      next_cursor: nextCursor,
      has_more: nextCursor.sequence < cursor.sequence,
      gap: false,
    }
  }
  function send(client, resource, body, force = false) {
    if (heldResources.has(resource.id) && !force) return
    body ??= pageFor(resource, client.after)
    client.response.write('event: content\ndata: ' + JSON.stringify(body) + '\n\n')
    client.after = body.next_cursor.sequence
  }
  function append(id, payload) {
    const resource = resources.get(id)
    resource.chunks.push({ cursor: { epoch, sequence: resource.chunks.length + 1 }, captured_at_ms: 1000, payload })
    publish(resource)
  }
  function publish(resource) {
    for (const client of resource.clients) send(client, resource)
    if (offline && page)
      pendingEmits.push(page.evaluate((value) => window.__contentFixture.publish(value), pageFor(resource)))
  }
  function complete(id, state = 'complete', finalSequence = resources.get(id).chunks.length) {
    const resource = resources.get(id)
    resource.state = state
    resource.finalSequence = finalSequence
    publish(resource)
  }
  async function holdResource(id, hold) {
    if (hold) heldResources.add(id)
    else heldResources.delete(id)
    if (offline) await page.evaluate(({ id, hold }) => window.__contentFixture.hold(id, hold), { id, hold })
    else if (!hold) for (const client of resources.get(id).clients) send(client, resources.get(id))
  }
  async function deliverPage(id, value) {
    if (offline) await page.evaluate(({ id, value }) => window.__contentFixture.deliver(id, value), { id, value })
    else for (const client of resources.get(id).clients) send(client, resources.get(id), value, true)
  }
  async function failStream(id) {
    if (offline) await page.evaluate((id) => window.__contentFixture.fail(id), id)
    else for (const client of resources.get(id).clients) client.response.destroy()
  }
  const log = (id, text) => append(id, { type: 'log', stream: 'stdout', text })
  function snapshot(rows, marker) {
    const attributes = {
      foreground: null,
      background: null,
      bold: false,
      dim: false,
      italic: false,
      underline: false,
      inverse: false,
    }
    return {
      rows,
      cols: 90,
      cursor_row: rows - 1,
      cursor_col: 0,
      cursor_visible: false,
      alternate_screen: false,
      bracketed_paste: false,
      application_cursor: false,
      cells: [
        { row: 0, col: 0, text: marker + ' first', attributes },
        { row: rows - 1, col: 0, text: marker + ' last', attributes },
      ],
    }
  }
  const historyPlain = 'red\nprogress 100%\n<script>literal-history</script>\nfirst\nUPDATED'
  log(
    'history-log',
    '\x1b[31mred\x1b[0m\nprogress 1%\r\x1b[2Kprogress 100%\n<script>literal-history</script>\nfirst\nsecond\n\x1b[1A\rUPDATED',
  )
  complete('history-log')
  complete('history-empty')
  for (let index = 0; index < 4; index++) complete('cache-' + index)
  append('history-terminal', { type: 'terminal', screen: snapshot(80, 'obsolete historical snapshot') })
  append('history-terminal', { type: 'terminal', screen: snapshot(24, 'historical snapshot') })
  complete('history-terminal')
  for (const [id, text] of [
    ['history-markdown', '# Completed report\n\n**Ready**\n'],
    ['history-json', '{"result":"json final","count":7}\n'],
    ['history-code', 'const finalAnswer = 42;\n'],
    [
      'history-diff',
      'diff --git a/final.txt b/final.txt\n--- a/final.txt\n+++ b/final.txt\n@@ -1 +1 @@\n-old final line\n+new final line\n',
    ],
  ]) {
    log(id, text)
    complete(id)
  }
  try {
    const entry = path.join(temporary, 'entry.ts')
    await fs.writeFile(
      entry,
      [
        "import { createApp, h, ref, computed, provide, nextTick } from 'vue'",
        "import { createPinia } from 'pinia'",
        "import { createRouter, createMemoryHistory } from 'vue-router'",
        "import { Terminal } from '@xterm/xterm'",
        'import AgenaContentOutput from ' + JSON.stringify('/@fs/' + component),
        'import { workspacePaneContextKey } from ' +
          JSON.stringify('/@fs/' + web + '/src/app/workspace/workspacePaneContext.ts'),
        'import { writeServerAuthTokenForBaseUrl } from ' +
          JSON.stringify('/@fs/' + web + '/src/lib/serverAuthToken.ts'),
        'import { i18n } from ' + JSON.stringify('/@fs/' + web + '/src/i18n/index.ts'),
        'import { terminalBufferPlainText } from ' + JSON.stringify('/@fs/' + web + '/src/lib/contentCopy.ts'),
        'import ' + JSON.stringify('/@fs/' + web + '/src/style.css'),
        "i18n.global.locale.value = 'en-US'",
        'const fixtureRouter=createRouter({history:createMemoryHistory(),routes:[{path:"/",component:{render:()=>null}}]})',
        'writeServerAuthTokenForBaseUrl(location.origin, ' + JSON.stringify(token) + ')',
        'const definitions = ' +
          JSON.stringify(
            Object.fromEntries([...resources.values()].map(({ id, kind, format }) => [id, { kind, format }])),
          ),
        'const terminals = new WeakMap(), renders = new WeakMap(), terminalIds = new WeakMap(), disposed = new WeakSet(), lifecycles = new Map()',
        'const life = id => { if(!lifecycles.has(id)) lifecycles.set(id,{opens:0,disposes:0,writes:0,disposedWhileWriting:0}); return lifecycles.get(id) }',
        'const originalOpen = Terminal.prototype.open, originalWrite = Terminal.prototype.write, originalDispose = Terminal.prototype.dispose',
        'Terminal.prototype.open = function(element) { const value = originalOpen.call(this, element); const id=element.closest("[data-output-case]")?.getAttribute("data-output-case"); terminalIds.set(this,id); if(id) life(id).opens++; terminals.set(element.closest(".content-terminal"), this); renders.set(this,0); this.onRender(()=>renders.set(this,renders.get(this)+1)); return value }',
        'Terminal.prototype.write = function(data,callback) { const id=terminalIds.get(this); if(id) life(id).writes++; return originalWrite.call(this,data,()=>{if(id) life(id).writes--; callback?.()}) }',
        'Terminal.prototype.dispose = function() { const id=terminalIds.get(this); if(id&&!disposed.has(this)){const value=life(id);value.disposes++;if(value.writes)value.disposedWhileWriting++;} disposed.add(this);return originalDispose.call(this) }',
        'const NativeResizeObserver=window.ResizeObserver, observedTargets=new Map()',
        'window.ResizeObserver=class extends NativeResizeObserver { observe(target,options){const id=target.closest(".content-terminal")?.closest("[data-output-case]")?.getAttribute("data-output-case");if(id){if(!observedTargets.has(this))observedTargets.set(this,new Map());observedTargets.get(this).set(target,id)}super.observe(target,options)} unobserve(target){observedTargets.get(this)?.delete(target);super.unobserve(target)} disconnect(){observedTargets.delete(this);super.disconnect()} }',
        'const observerCount=id=>[...observedTargets.values()].reduce((count,targets)=>count+[...targets.values()].filter(value=>value===id).length,0)',
        "const visible = ref(true), offscreen = ref(false), width = ref('780px'), extraIds=ref([])",
        "const surface = id => document.querySelector('[data-output-case=\"'+id+'\"] .content-terminal')",
        'const emulator = id => terminals.get(surface(id))',
        'const frame = () => new Promise(resolve => requestAnimationFrame(resolve))',
        'window.__outputProbe = {',
        '  ids:Object.keys(definitions),',
        '  settle: async () => { await nextTick(); for (let i=0;i<5;i++) await frame() },',
        '  mountOffscreen: async () => { offscreen.value = true; await nextTick() },',
        '  mountExtras: async ids => { extraIds.value=[...new Set([...extraIds.value,...ids])];await nextTick() },',
        '  visible: async value => { visible.value = value; await nextTick() },',
        "  width: async value => { width.value = value+'px'; await nextTick() },",
        '  scroll: (id, line) => emulator(id).scrollToLine(line),',
        '  scrollToBottom: id => emulator(id).scrollToBottom(),',
        '  select: id => { const t = emulator(id); t.select(0, t.buffer.active.baseY, 6) },',
        '  clearSelection: id => { emulator(id).clearSelection();document.getSelection()?.removeAllRanges() },',
        "  followLabel: i18n.global.t('content.follow'),",
        "  copyLabel:i18n.global.t('content.copy'), interruptedLabel:i18n.global.t('content.interrupted'),",
        '  lifecycle:id=>({...life(id),observers:observerCount(id)}),',
        '  staticState:id=>{const element=document.querySelector("[data-output-case=\\\""+id+"\\\"] [data-content-static]");if(!element)return null;return {display:"static",format:element.getAttribute("data-format"),height:element.getBoundingClientRect().height,width:element.clientWidth,scrollHeight:element.scrollHeight,scrollTop:element.scrollTop,text:element.textContent,top:element.getBoundingClientRect().top,toolbar:element.closest("[data-content-output]")?.querySelector(".part-toolbar")?.textContent}},',
        '  state: id => {',
        '    const element = surface(id), terminal = emulator(id); if (!element || !terminal || disposed.has(terminal)) return null;',
        '    const buffer = terminal.buffer.active, visibleLines = [];',
        "    for (let i=0;i<terminal.rows;i++) visibleLines.push(buffer.getLine(buffer.viewportY+i)?.translateToString(true)||'');",
        '    return {height:element.getBoundingClientRect().height, width:element.clientWidth, scrollHeight:element.scrollHeight, scrollTop:element.scrollTop,',
        '      layers:[...element.querySelectorAll(".xterm,.xterm-screen,.xterm-viewport,.xterm-scrollable-element,.xterm-scroll-area,.xterm-rows")].map(node=>({class:node.className,height:node.offsetHeight,scrollHeight:node.scrollHeight,scrollTop:node.scrollTop,top:node.getBoundingClientRect().top,bottom:node.getBoundingClientRect().bottom})),',
        '      screenHeight:element.querySelector(".xterm-screen")?.getBoundingClientRect().height, screenWidth:element.querySelector(".xterm-screen")?.getBoundingClientRect().width,',
        '      screenBottom:element.querySelector(".xterm-screen")?.getBoundingClientRect().bottom, visibleText:visibleLines.join("\\n"),',
        '      renderedText:element.querySelector(".xterm-rows")?.textContent, toolbar:element.closest("[data-content-output]")?.querySelector(".part-toolbar")?.textContent,',
        '      rows:terminal.rows, cols:terminal.cols, convertEol:terminal.options.convertEol, renders:renders.get(terminal), baseY:buffer.baseY, viewportY:buffer.viewportY, text:terminalBufferPlainText(buffer),',
        '      selected:terminal.getSelection(), top:element.getBoundingClientRect().top};',
        '  },',
        '}',
        'const app = createApp({ setup() {',
        '  provide(workspacePaneContextKey, {isVisible:computed(()=>visible.value),isFocused:computed(()=>true),windowId:computed(()=>"fixture"),route:computed(()=>fixtureRouter.currentRoute.value),navigate:(to,replace)=>replace?fixtureRouter.replace(to):fixtureRouter.push(to)})',
        '  const output = id => h("section", {key:id,"data-output-case":id}, [h("h2", id),',
        '    h(AgenaContentOutput, {key:id,resource:{resource_id:id,kind:definitions[id].kind},format:definitions[id].format,sessionId:"48"})])',
        '  return () => h("main", {style:{width:width.value,display:visible.value?"block":"none"}}, [',
        '    output("short"), output("long"), h("div",{style:{height:"1800px"},"data-spacer":true}),',
        '    ...(offscreen.value?[output("offscreen")]:[]), output("terminal"), h("div",{id:"after-output"},"After output"),...extraIds.value.map(output),',
        '  ])',
        '}})',
        "app.use(createPinia()).use(fixtureRouter).use(i18n).mount('#app')",
      ].join('\n'),
    )
    server = await createServer({
      root: web,
      configFile: path.join(web, 'vite.config.ts'),
      logLevel: 'warn',
      server: {
        host: '127.0.0.1',
        port: 0,
        middlewareMode: offline,
        hmr: offline ? false : undefined,
        fs: { allow: [repo, temporary] },
      },
      plugins: [
        {
          name: 'content-output-probe-isolation',
          enforce: 'pre',
          transform(code, id) {
            if (baseline && id.split('?')[0] === component && !id.includes('?')) return baseline
          },
          configureServer(dev) {
            dev.middlewares.use(async (req, res, next) => {
              const url = new URL(req.url, 'http://fixture')
              const json = (value) => {
                res.setHeader('Content-Type', 'application/json')
                res.end(JSON.stringify(value))
              }
              if (/^\/(api|auth|health|plugin-assets)(\/|$)/.test(url.pathname)) {
                requests.push({ method: req.method, path: url.pathname, after: url.searchParams.get('after') })
                const match = /^\/api\/v1\/sessions\/48\/content\/([^/]+)(\/stream)?$/.exec(url.pathname)
                const resource = match && resources.get(match[1])
                if (resource && req.method === 'GET' && req.headers.authorization === 'Bearer ' + token) {
                  const after = Number(url.searchParams.get('after') || 0)
                  if (match[2]) {
                    res.setHeader('Content-Type', 'text/event-stream')
                    res.setHeader('Cache-Control', 'no-store')
                    const client = { response: res, after }
                    resource.clients.add(client)
                    res.on('close', () => resource.clients.delete(client))
                    return send(client, resource)
                  }
                  return json(pageFor(resource, after))
                }
                unexpected.push({ method: req.method, path: url.pathname })
                res.statusCode = 404
                return json({ error: 'Unavailable in isolated content fixture' })
              }
              if (url.pathname !== '/__content-output-probe') return next()
              try {
                const html =
                  '<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Content output regression</title></head><body><div id="app"></div><style>html,body,#app{height:auto!important;min-height:100%;overflow:visible!important;margin:0}main{max-width:calc(100vw - 32px);margin:16px}section{margin-bottom:12px}h2{font-size:12px;height:18px;margin:0}</style><script type="module" src="/@fs/' +
                  entry +
                  '"></script></body></html>'
                res.setHeader('Content-Type', 'text/html')
                res.end(await dev.transformIndexHtml('/__content-output-probe', html))
              } catch (error) {
                next(error)
              }
            })
          },
        },
      ],
    })
    if (!offline) await server.listen()
    const origin = offline ? 'https://content-output.test' : 'http://127.0.0.1:' + server.httpServer.address().port
    browser = await chromium.launch({
      executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
      headless: true,
      args: ['--no-sandbox', '--disable-dev-shm-usage'],
    })
    const context = await browser.newContext({
      viewport: { width: 1000, height: 800 },
      serviceWorkers: 'block',
      permissions: ['clipboard-read', 'clipboard-write'],
    })
    page = await context.newPage()
    page.setDefaultTimeout(10000)
    page.setDefaultNavigationTimeout(60000)
    page.on('pageerror', (error) => errors.push(error.stack || error.message))
    if (offline) {
      await page.addInitScript(
        ({ initial, token }) => {
          const states = new Map(initial.map((value) => [value.resource.resource_id, value]))
          const clients = new Map()
          const held = new Set()
          const requests = []
          const encoder = new TextEncoder()
          const originalFetch = window.fetch.bind(window)
          const filtered = (value, after) => ({
            ...value,
            chunks: value.chunks.filter((chunk) => chunk.cursor.sequence > after),
          })
          const emit = (client, value) => {
            client.controller.enqueue(
              encoder.encode('event: content\ndata: ' + JSON.stringify(filtered(value, client.after)) + '\n\n'),
            )
            client.after = value.next_cursor.sequence
          }
          window.__contentFixture = {
            requests,
            hold(id, hold) {
              if (hold) held.add(id)
              else {
                held.delete(id)
                for (const client of clients.get(id) || []) emit(client, states.get(id))
              }
            },
            deliver(id, value) {
              for (const client of clients.get(id) || []) emit(client, value)
            },
            fail(id) {
              const current = [...(clients.get(id) || [])]
              clients.get(id)?.clear()
              for (const client of current) client.controller.error(new Error('Fixture content stream interrupted'))
            },
            publish(value) {
              states.set(value.resource.resource_id, value)
              if (!held.has(value.resource.resource_id))
                for (const client of clients.get(value.resource.resource_id) || []) emit(client, value)
            },
          }
          window.fetch = async (input, init = {}) => {
            const url = new URL(typeof input === 'string' ? input : input.url, location.href)
            const match = /^\/api\/v1\/sessions\/48\/content\/([^/]+)(\/stream)?$/.exec(url.pathname)
            if (url.origin !== location.origin || !match) return originalFetch(input, init)
            const value = states.get(match[1]),
              after = Number(url.searchParams.get('after') || 0)
            if (!value || new Headers(init.headers).get('authorization') !== 'Bearer ' + token)
              return new Response('', { status: 403 })
            requests.push({ method: init.method || 'GET', path: url.pathname, after })
            if (!match[2])
              return new Response(JSON.stringify(filtered(value, after)), {
                headers: { 'content-type': 'application/json' },
              })
            let client
            const remove = () => clients.get(match[1])?.delete(client)
            return new Response(
              new ReadableStream({
                start(controller) {
                  client = { controller, after }
                  if (!clients.has(match[1])) clients.set(match[1], new Set())
                  clients.get(match[1]).add(client)
                  init.signal?.addEventListener(
                    'abort',
                    () => {
                      remove()
                      try {
                        controller.error(init.signal.reason)
                      } catch {}
                    },
                    { once: true },
                  )
                  if (!held.has(match[1])) emit(client, value)
                },
                cancel: remove,
              }),
              { headers: { 'content-type': 'text/event-stream' } },
            )
          }
        },
        { initial: [...resources.values()].map((resource) => pageFor(resource)), token },
      )
    }
    await page.route('**/*', async (route) => {
      const url = new URL(route.request().url())
      if (url.origin !== origin) {
        external.push(url.href)
        return route.abort()
      }
      if (!offline) return route.continue()
      try {
        if (/^\/(api|auth|health|plugin-assets)(\/|$)/.test(url.pathname)) {
          unexpected.push({ method: route.request().method(), path: url.pathname })
          return route.fulfill({ status: 404, body: 'Unavailable fixture request' })
        }
        if (url.pathname === '/__content-output-probe') {
          const html =
            '<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Content output regression</title></head><body><div id="app"></div><style>html,body,#app{height:auto!important;min-height:100%;overflow:visible!important;margin:0}main{max-width:calc(100vw - 32px);margin:16px}section{margin-bottom:12px}h2{font-size:12px;height:18px;margin:0}</style><script type="module" src="/@fs/' +
            entry +
            '"></script></body></html>'
          return route.fulfill({ contentType: 'text/html', body: html })
        }
        if (url.pathname === '/@vite/client') {
          const body =
            'const styles=new Map();export function updateStyle(id,content){let style=styles.get(id);if(!style){style=document.createElement("style");document.head.appendChild(style);styles.set(id,style)}style.textContent=content}export function removeStyle(id){styles.get(id)?.remove();styles.delete(id)}export function createHotContext(){return {accept(){},dispose(){},prune(){},invalidate(){},on(){},off(){},send(){},data:{}}}export function injectQuery(url,query){return url+(url.includes("?")?"&":"?")+query}'
          return route.fulfill({ contentType: 'text/javascript', body })
        }
        if (/\.(woff2?|ttf|otf|svg|png|ico)$/.test(url.pathname)) {
          if (url.pathname === '/favicon.ico') return route.fulfill({ status: 204 })
          const file = url.pathname.startsWith('/@fs/') ? url.pathname.slice(4) : path.join(web, url.pathname)
          assert.ok(
            file.startsWith(web + path.sep) || file.startsWith(temporary + path.sep),
            'fixture asset stays inside its source tree',
          )
          return route.fulfill({
            body: await fs.readFile(file),
            contentType: url.pathname.endsWith('.woff2') ? 'font/woff2' : 'application/octet-stream',
          })
        }
        const moduleUrl = url.pathname.startsWith('/@id/')
          ? url.pathname.slice(5).replace('__x00__', '\0')
          : url.pathname
        const result = await server.transformRequest(moduleUrl + url.search)
        assert.ok(result, 'Vite transforms ' + url.pathname)
        return route.fulfill({ contentType: 'text/javascript', body: result.code })
      } catch (error) {
        errors.push(error.stack)
        return route.fulfill({ status: 500, body: error.message })
      }
    })
    await page.goto(origin + '/__content-output-probe', { waitUntil: 'domcontentloaded' })
    await page.waitForFunction(() => Boolean(window.__outputProbe?.state('long')), null, { timeout: 60000 })
    await page.evaluate(() => document.fonts.ready)
    const settle = async () => {
      await Promise.all(pendingEmits.splice(0))
      await page.evaluate(() => window.__outputProbe.settle())
    }
    const state = (id) => page.evaluate((id) => window.__outputProbe.state(id), id)
    async function hasText(id, marker) {
      await Promise.all(pendingEmits.splice(0))
      await page.waitForFunction(({ id, marker }) => window.__outputProbe.state(id)?.text.includes(marker), {
        id,
        marker,
      })
      await settle()
    }
    async function bounded(id, label, max = 384) {
      await settle()
      const sample = await page.evaluate(async (id) => {
        let value = window.__outputProbe.state(id),
          peakHeight = value.height,
          peakRows = value.rows
        for (let index = 0; index < 8; index++) {
          await new Promise((resolve) => requestAnimationFrame(resolve))
          value = window.__outputProbe.state(id)
          peakHeight = Math.max(peakHeight, value.height)
          peakRows = Math.max(peakRows, value.rows)
        }
        return { value, peakHeight, peakRows }
      }, id)
      const { value, peakHeight, peakRows } = sample
      geometry.push({
        label,
        id,
        height: value.height,
        peakHeight,
        screenHeight: value.screenHeight,
        rows: value.rows,
        peakRows,
        width: value.width,
      })
      check(
        value.height > 0 && peakHeight <= max + 1,
        label + ': peak output height ' + peakHeight + 'px stays within ' + max + 'px',
      )
      if (id !== 'terminal') {
        check(value.rows >= 1 && value.rows <= 12, label + ': emulator keeps 1–12 visible rows')
        check(value.screenHeight <= value.height + 1, label + ': emulator screen fits inside the log panel')
        check(value.screenWidth <= value.width + 1, label + ': emulator columns fit the log width')
      }
      return value
    }
    async function follow(id) {
      const label = await page.evaluate(() => window.__outputProbe.followLabel)
      await page
        .locator('[data-output-case="' + id + '"] button')
        .filter({ hasText: label })
        .click()
      await settle()
    }

    const staticPanel = (id) => page.locator('[data-output-case="' + id + '"] [data-content-static]')
    const staticState = (id) => page.evaluate((id) => window.__outputProbe.staticState(id), id)
    const lifecycle = (id) => page.evaluate((id) => window.__outputProbe.lifecycle(id), id)
    async function mountExtras(ids) {
      await page.evaluate((ids) => window.__outputProbe.mountExtras(ids), ids)
      await settle()
    }
    async function finalized(id, marker, expectedOpens) {
      await page.waitForFunction(
        ({ id, marker }) => {
          const value = window.__outputProbe.staticState(id)
          return Boolean(value) && (!marker || value.text.includes(marker))
        },
        { id, marker },
      )
      await settle()
      const value = await staticState(id),
        ownership = await lifecycle(id)
      check(value.height > 0 && value.height <= 385, id + ': final result stays within 384px')
      check(
        (await page.locator('[data-output-case="' + id + '"] .xterm').count()) === 0,
        id + ': final result removes the live terminal DOM',
      )
      check(ownership.disposes === ownership.opens, id + ': every opened terminal is disposed')
      check(
        ownership.writes === 0 && ownership.disposedWhileWriting === 0,
        id + ': finalization waits for pending writes',
      )
      check(ownership.observers === 0, id + ': finalization disconnects terminal resize observers')
      if (expectedOpens !== undefined)
        check(ownership.opens === expectedOpens, id + ': terminal open count is ' + expectedOpens)
      geometry.push({
        label: 'final ' + id,
        id,
        display: 'static',
        format: value.format,
        height: value.height,
        scrollHeight: value.scrollHeight,
      })
      return value
    }
    async function staticTailVisible(id, marker) {
      await staticPanel(id).evaluate((element) => {
        element.scrollTop = element.scrollHeight
      })
      await settle()
      const visible = await staticPanel(id).evaluate((element, marker) => {
        const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT)
        let node
        while ((node = walker.nextNode())) {
          const offset = node.textContent.lastIndexOf(marker)
          if (offset < 0) continue
          const range = document.createRange()
          range.setStart(node, offset)
          range.setEnd(node, offset + marker.length)
          const line = range.getBoundingClientRect(),
            panel = element.getBoundingClientRect()
          return line.height > 0 && line.top >= panel.top - 1 && line.bottom <= panel.bottom + 1
        }
        return false
      }, marker)
      check(visible, id + ': final result can scroll to its last line')
    }
    async function copyResult(id) {
      const label = await page.evaluate(() => window.__outputProbe.copyLabel)
      await page
        .locator('[data-output-case="' + id + '"] button')
        .filter({ hasText: label })
        .click()
      await page.waitForFunction(
        (id) => !document.querySelector('[data-output-case="' + id + '"] button[aria-busy="true"]'),
        id,
      )
      return page.evaluate(() => navigator.clipboard.readText())
    }

    stage('short and long log geometry')
    log('short', 'short marker\n')
    log('long', 'visible marker\n')
    await hasText('short', 'short marker')
    await hasText('long', 'visible marker')
    await bounded('short', 'one-line log remains compact', 80)

    stage('mounting and streaming outside the viewport')
    if (hiddenMount) {
      await page.evaluate(() => window.__outputProbe.visible(false))
      await settle()
    }
    await page.evaluate(() => window.__outputProbe.mountOffscreen())
    if (hiddenMount) {
      // Hidden panes defer subscribing and opening xterm until they become visible.
      await page.locator('[data-output-case="offscreen"] .content-terminal').waitFor({ state: 'attached' })
    } else {
      await page.waitForFunction(() => Boolean(window.__outputProbe.state('offscreen')))
    }
    await settle()
    if (hiddenMount) {
      await page.evaluate(() => window.__outputProbe.visible(true))
      await settle()
      await page.waitForFunction(() => Boolean(window.__outputProbe.state('offscreen')))
    }
    check((await state('offscreen')).top > 800, 'offscreen fixture is below the viewport')
    for (let index = 0; index < 12; index++) {
      log(
        'offscreen',
        Array.from({ length: 18 }, (_, line) => 'offscreen ' + index + ' line ' + line).join('\n') + '\n',
      )
      await bounded('offscreen', 'offscreen streaming batch ' + index)
      await hasText('offscreen', 'offscreen ' + index + ' line 17')
    }
    await page.locator('[data-output-case="offscreen"]').scrollIntoViewIfNeeded()
    await bounded('offscreen', 'offscreen log scrolled into view')
    check(
      (await state('offscreen')).visibleText.includes('offscreen 11 line 17'),
      'latest offscreen output is visible after scrolling back',
    )
    check(
      (await state('offscreen')).renderedText.includes('offscreen 11 line 17'),
      'offscreen output reaches the real rendered terminal rows',
    )

    stage('visible streaming, scrolling back, and width changes')
    await page.locator('[data-output-case="long"]').scrollIntoViewIfNeeded()
    log(
      'long',
      Array.from({ length: 220 }, (_, index) => 'long line ' + index + ' ' + 'wrap '.repeat(30)).join('\n') + '\n',
    )
    await hasText('long', 'long line 219')
    await bounded('long', 'long log')
    await page.evaluate(() => window.__outputProbe.width(320))
    await bounded('long', 'narrow log')
    await page.evaluate(() => window.__outputProbe.width(780))
    await bounded('long', 'wide log')
    for (let index = 0; index < 24; index++) log('long', 'burst marker ' + index + '\n')
    await hasText('long', 'burst marker 23')
    await bounded('long', 'continuous streaming burst')
    check((await state('long')).renderedText.includes('burst marker 23'), 'continuous streaming paints its latest line')
    await page.locator('#after-output').scrollIntoViewIfNeeded()
    for (let index = 0; index < 8; index++) {
      log('long', 'while away ' + index + '\n')
      await hasText('long', 'while away ' + index)
      await bounded('long', 'previously visible log offscreen ' + index)
    }
    await page.locator('[data-output-case="long"]').scrollIntoViewIfNeeded()
    await bounded('long', 'long log returned to viewport')
    const idle = await page.evaluate(async () => {
      const first = window.__outputProbe.state('long')
      for (let index = 0; index < 15; index++) await new Promise((resolve) => requestAnimationFrame(resolve))
      const last = window.__outputProbe.state('long')
      return {
        first: { height: first.height, rows: first.rows, renders: first.renders },
        last: { height: last.height, rows: last.rows, renders: last.renders },
      }
    })
    check(
      JSON.stringify(idle.first) === JSON.stringify(idle.last),
      'idle output has stable geometry and stops rendering',
    )

    stage('pause, selection, follow and pane visibility recovery')
    await page.evaluate(() => window.__outputProbe.scroll('long', 0))
    await settle()
    const paused = await state('long')
    check(paused.viewportY < paused.baseY, 'scrolling back pauses the output viewport')
    log('long', 'paused append marker\n')
    await settle()
    check(!(await state('long')).text.includes('paused append marker'), 'paused log keeps the displayed history stable')
    await follow('long')
    await hasText('long', 'paused append marker')
    check((await state('long')).viewportY === (await state('long')).baseY, 'follow resumes at the latest output')
    await page.evaluate(() => window.__outputProbe.select('long'))
    check(Boolean((await state('long')).selected), 'terminal text selection is active')
    log('long', 'selected append marker\n')
    await settle()
    check(!(await state('long')).text.includes('selected append marker'), 'selection preserves the displayed history')
    await follow('long')
    await hasText('long', 'selected append marker')
    check(
      (await state('long')).visibleText.includes('selected append marker'),
      'following makes the latest line visible',
    )
    check(
      (await state('long')).renderedText.includes('selected append marker'),
      'following paints the latest line in the terminal DOM',
    )
    await page.evaluate(() => window.__outputProbe.visible(false))
    await settle()
    log('long', 'hidden pane marker\n')
    await page.evaluate(() => window.__outputProbe.visible(true))
    await hasText('long', 'hidden pane marker')
    await bounded('long', 'hidden pane restored')
    check((await state('long')).visibleText.includes('hidden pane marker'), 'restored pane displays the latest line')
    check(
      (await state('long')).renderedText.includes('hidden pane marker'),
      'restored pane paints the latest line in the terminal DOM',
    )
    if (offline) requests.push(...(await page.evaluate(() => window.__contentFixture.requests)))
    check(
      requests.filter((request) => request.path.endsWith('/long/stream')).some((request) => Number(request.after) > 0),
      'pane visibility resumes the stream from its accepted cursor',
    )
    if (process.env.PROBE_SCREENSHOT_DIR) {
      await fs.mkdir(process.env.PROBE_SCREENSHOT_DIR, { recursive: true })
      await page.evaluate(() => window.scrollTo(0, 0))
      await settle()
      await page.screenshot({ path: path.join(process.env.PROBE_SCREENSHOT_DIR, 'logs.png') })
    }

    stage('terminal snapshots preserve producer dimensions within a bounded panel')
    await page.locator('[data-output-case="terminal"]').scrollIntoViewIfNeeded()
    for (const [index, rows] of [24, 80, 24].entries()) {
      const marker = 'snapshot-' + index
      append('terminal', { type: 'terminal', screen: snapshot(rows, marker) })
      await hasText('terminal', marker + ' last')
      const current = await bounded('terminal', 'terminal snapshot ' + rows + ' rows')
      check(current.rows === rows, 'terminal snapshot retains ' + rows + ' producer rows')
      check(current.cols === 90, 'terminal snapshot retains 90 producer columns')
      check(current.text.includes(marker + ' first'), 'terminal snapshot keeps its first row')
      await page.locator('[data-output-case="terminal"] .content-terminal').evaluate((element) => {
        element.scrollTop = element.scrollHeight
      })
      await settle()
      const scrolled = await state('terminal')
      check(
        scrolled.screenBottom <= scrolled.top + scrolled.height + 1 && scrolled.screenBottom > scrolled.top,
        'terminal snapshot last row can be scrolled into the panel at ' + rows + ' rows',
      )
    }
    if (process.env.PROBE_SCREENSHOT_DIR)
      await page.screenshot({ path: path.join(process.env.PROBE_SCREENSHOT_DIR, 'terminal.png') })
    check(errors.length === 0, 'no browser exceptions')
    check(unexpected.length === 0, 'all application requests use the fixture contract')
    check(external.length === 0, 'no external requests')
    const streamingAssertions = checks.length

    stage('final results: status-only completion and last-write growth')
    check((await lifecycle('short')).observers > 0, 'running output owns a resize observer')
    log('short', '\x1b[32mstate-only marker\x1b[0m\n')
    await hasText('short', 'state-only marker')
    complete('short')
    const shortFinal = await finalized('short', 'state-only marker', 1)
    check(shortFinal.format === 'plain' && !shortFinal.text.includes('\x1b'), 'completed logs show decoded plain text')
    check(
      (await copyResult('short')) === 'short marker\nstate-only marker\n',
      'completed log copies its full decoded source',
    )

    await mountExtras(['short-final', 'draining', 'manual-follow', 'interrupted', 'empty-live'])
    log('short-final', 'growth first')
    await hasText('short-final', 'growth first')
    check((await state('short-final')).rows === 1, 'final-write growth starts with a one-row terminal')
    log('short-final', '\ngrowth tail')
    complete('short-final')
    const growth = await finalized('short-final', 'growth tail', 1)
    check(growth.text.includes('growth first'), 'completion retains both rows after final-write growth')

    stage('final results wait for the complete cursor and all pending output')
    log('draining', 'first page\n')
    await hasText('draining', 'first page')
    complete('draining', 'complete', 3)
    await settle()
    check(
      (await staticPanel('draining').count()) === 0 && Boolean(await state('draining')),
      'complete metadata with an unread final cursor retains the live output',
    )
    log('draining', 'middle page\n')
    await hasText('draining', 'middle page')
    check((await staticPanel('draining').count()) === 0, 'intermediate completed pages do not finalize early')
    log('draining', 'last page marker\n')
    const drained = await finalized('draining', 'last page marker', 1)
    check(
      drained.text.includes('first page') && drained.text.includes('middle page'),
      'final result includes every completed page',
    )

    stage('completion preserves paused and selected output until the reader resumes')
    await page.locator('[data-output-case="long"]').scrollIntoViewIfNeeded()
    await page.evaluate(() => window.__outputProbe.scroll('long', 0))
    await settle()
    const pausedFinal = await state('long')
    log('long', 'paused completion tail\n')
    complete('long')
    await settle()
    check((await staticPanel('long').count()) === 0, 'completion keeps a manually paused terminal open')
    check((await state('long')).viewportY === pausedFinal.viewportY, 'completion preserves the paused scroll position')
    await follow('long')
    await finalized('long', 'paused completion tail', 1)
    await staticTailVisible('long', 'paused completion tail')

    await page.locator('[data-output-case="offscreen"]').scrollIntoViewIfNeeded()
    await page.evaluate(() => window.__outputProbe.select('offscreen'))
    const selectedFinal = (await state('offscreen')).selected
    check(Boolean(selectedFinal), 'completion fixture begins with selected terminal text')
    log('offscreen', 'selected completion tail\n')
    complete('offscreen')
    await settle()
    check(
      (await staticPanel('offscreen').count()) === 0 && (await state('offscreen')).selected === selectedFinal,
      'completion retains an active selection and its terminal',
    )
    await page.evaluate(() => window.__outputProbe.clearSelection('offscreen'))
    await finalized('offscreen', 'selected completion tail', 1)
    await staticTailVisible('offscreen', 'selected completion tail')

    log('manual-follow', Array.from({ length: 40 }, (_, index) => 'manual row ' + index).join('\n') + '\n')
    await hasText('manual-follow', 'manual row 39')
    await page.locator('[data-output-case="manual-follow"]').scrollIntoViewIfNeeded()
    await page.evaluate(() => window.__outputProbe.scroll('manual-follow', 0))
    log('manual-follow', 'manual completion tail\n')
    complete('manual-follow')
    await settle()
    check((await staticPanel('manual-follow').count()) === 0, 'manual-bottom fixture remains paused on completion')
    await page.evaluate(() => window.__outputProbe.scrollToBottom('manual-follow'))
    await finalized('manual-follow', 'manual completion tail', 1)

    stage('terminal snapshots, interruption and empty output finalize')
    complete('terminal')
    const terminalFinal = await finalized('terminal', 'snapshot-2 last', 1)
    check(
      terminalFinal.text.includes('snapshot-2 first') && !terminalFinal.text.includes('snapshot-1 first'),
      'completed terminal contains its final screen without old scrollback',
    )
    await staticTailVisible('terminal', 'snapshot-2 last')
    check((await copyResult('terminal')) === terminalFinal.text, 'completed terminal copies its retained final screen')
    log('interrupted', 'interrupted final marker\n')
    await hasText('interrupted', 'interrupted final marker')
    complete('interrupted', 'interrupted')
    const interrupted = await finalized('interrupted', 'interrupted final marker', 1)
    check(
      interrupted.toolbar.includes(await page.evaluate(() => window.__outputProbe.interruptedLabel)),
      'interrupted output keeps its interrupted status in the static view',
    )
    await page.waitForFunction(
      () =>
        document.querySelector('[data-output-case="empty-live"] [data-content-output]')?.getAttribute('aria-busy') ===
        'false',
    )
    const emptyOpens = (await lifecycle('empty-live')).opens
    complete('empty-live')
    const emptyFinal = await finalized('empty-live', null, emptyOpens)
    check(emptyFinal.text.trim() === '', 'an active source can complete with no output')

    stage('completed history renders directly without opening a terminal')
    const historyIds = [...resources.keys()].filter((id) => id.startsWith('history-'))
    await mountExtras(historyIds)
    const historical = await finalized('history-log', 'UPDATED', 0)
    check(
      historical.text === historyPlain,
      'completed history decodes ANSI, carriage returns and multiline cursor rewrites',
    )
    check(!historical.text.includes('second'), 'multiline cursor movement replaces the intended previous row')
    check(
      (await staticPanel('history-log').locator('script').count()) === 0,
      'plain final output displays markup as literal text',
    )
    check((await copyResult('history-log')) === historyPlain, 'completed history copies the same decoded text')
    const selectedStatic = await staticPanel('history-log').evaluate((element) => {
      const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT)
      let node
      while ((node = walker.nextNode())) {
        const offset = node.textContent.indexOf('UPDATED')
        if (offset < 0) continue
        const range = document.createRange()
        range.setStart(node, offset)
        range.setEnd(node, offset + 7)
        const selection = document.getSelection()
        selection.removeAllRanges()
        selection.addRange(range)
        return selection.toString()
      }
      return ''
    })
    check(selectedStatic === 'UPDATED', 'static output supports native text selection')
    check((await copyResult('history-log')) === 'UPDATED', 'static Copy preserves the native selected text')
    const followLabel = await page.evaluate(() => window.__outputProbe.followLabel)
    check(
      (await page.locator('[data-output-case="history-log"] button').filter({ hasText: followLabel }).count()) === 0,
      'static text selection does not expose a live Follow action',
    )
    await page.evaluate(() => document.getSelection()?.removeAllRanges())
    const emptyHistory = await finalized('history-empty', null, 0)
    check(emptyHistory.text.trim() === '', 'empty completed history needs no terminal DOM')
    const historicalTerminal = await finalized('history-terminal', 'historical snapshot last', 0)
    check(
      !historicalTerminal.text.includes('obsolete historical snapshot'),
      'completed terminal history displays only its latest snapshot',
    )
    for (const [id, marker, format] of [
      ['history-markdown', 'Completed report', 'markdown'],
      ['history-json', 'json final', 'json'],
      ['history-code', 'finalAnswer', 'code'],
      ['history-diff', 'new final line', 'diff'],
    ]) {
      const value = await finalized(id, marker, 0)
      check(value.format === format, id + ': preserves the requested final format')
    }
    await staticPanel('history-markdown').scrollIntoViewIfNeeded()
    await staticPanel('history-markdown').locator('h1').waitFor()
    check(
      (await staticPanel('history-markdown').locator('h1').textContent()) === 'Completed report' &&
        (await staticPanel('history-markdown').locator('strong').textContent()) === 'Ready',
      'completed Markdown renders a heading and emphasis',
    )
    check(
      JSON.parse(await staticPanel('history-json').locator('pre code').textContent()).count === 7,
      'completed JSON renders readable structured text',
    )
    check(
      (await staticPanel('history-code').locator('pre code').textContent()).includes('const finalAnswer = 42;'),
      'completed code uses a code block',
    )
    check(
      (await staticState('history-diff')).text.includes('old final line'),
      'completed diff retains both removed and added lines',
    )

    stage('completed snapshots survive cache eviction, partial reloads and stream failure')
    // Twenty real subscriptions exceed the shared sixteen-entry idle cache.
    // Hiding their pane releases them through the normal public lifecycle.
    const cacheIds = ['cache-0', 'cache-1', 'cache-2', 'cache-3']
    await mountExtras(cacheIds)
    for (const id of cacheIds) await finalized(id, null, 0)
    const terminalRequests = async () =>
      offline
        ? page.evaluate(() =>
            window.__contentFixture.requests.filter((request) => request.path.endsWith('/terminal/stream')),
          )
        : requests.filter((request) => request.path.endsWith('/terminal/stream'))
    const requestsBefore = (await terminalRequests()).length
    await holdResource('terminal', true)
    const opensBefore = await page.evaluate(() =>
      window.__outputProbe.ids.map((id) => window.__outputProbe.lifecycle(id).opens),
    )
    await page.evaluate(() => window.__outputProbe.visible(false))
    await settle()
    await page.evaluate(() => window.__outputProbe.visible(true))
    for (let attempt = 0; attempt < 20 && (await terminalRequests()).length === requestsBefore; attempt++)
      await settle()
    const requestsAfterBootstrap = (await terminalRequests()).length
    check(requestsAfterBootstrap > requestsBefore, 'pane resume recreates the evicted terminal content subscription')
    check(
      (await staticState('terminal'))?.text === terminalFinal.text,
      'empty cache bootstrap preserves the complete static snapshot',
    )
    check((await copyResult('terminal')) === terminalFinal.text, 'copy works before an evicted resource has reloaded')
    const partial = pageFor(resources.get('terminal'))
    partial.chunks = partial.chunks.slice(0, 1)
    partial.next_cursor = partial.chunks[0].cursor
    partial.has_more = true
    await deliverPage('terminal', partial)
    await settle()
    check(
      (await staticState('terminal'))?.text === terminalFinal.text,
      'a partial page for the same final cursor preserves the complete snapshot',
    )
    check(
      (await page.locator('[data-output-case="terminal"] [data-content-output]').getAttribute('data-content-state')) ===
        'complete',
      'partial reload preserves completion metadata',
    )
    await failStream('terminal')
    await settle()
    check(
      (await staticState('terminal'))?.text === terminalFinal.text,
      'a failed partial reload retains the complete snapshot',
    )
    check((await copyResult('terminal')) === terminalFinal.text, 'copy remains available after a failed partial reload')
    await holdResource('terminal', false)
    for (let attempt = 0; attempt < 20 && (await terminalRequests()).length === requestsAfterBootstrap; attempt++)
      await settle()
    check(
      (await terminalRequests()).some((request) => Number(request.after) === 1),
      'reloading retries from its accepted partial-page cursor',
    )
    await settle()
    await page.evaluate(() => window.__outputProbe.width(320))
    await settle()
    await page.evaluate(() => window.__outputProbe.width(780))
    await settle()
    const opensAfter = await page.evaluate(() =>
      window.__outputProbe.ids.map((id) => window.__outputProbe.lifecycle(id).opens),
    )
    check(
      JSON.stringify(opensAfter) === JSON.stringify(opensBefore),
      'completed outputs stay static across pane and width changes',
    )
    check((await page.locator('.xterm').count()) === 0, 'all completed outputs release their live terminal elements')
    await staticTailVisible('long', 'paused completion tail')
    if (process.env.PROBE_SCREENSHOT_DIR) {
      await page.locator('[data-output-case="long"]').scrollIntoViewIfNeeded()
      await page.screenshot({ path: path.join(process.env.PROBE_SCREENSHOT_DIR, 'final-logs.png') })
      await page.locator('[data-output-case="terminal"]').scrollIntoViewIfNeeded()
      await page.screenshot({ path: path.join(process.env.PROBE_SCREENSHOT_DIR, 'final-terminal.png') })
      await page.locator('[data-output-case="history-markdown"]').scrollIntoViewIfNeeded()
      await page.screenshot({ path: path.join(process.env.PROBE_SCREENSHOT_DIR, 'final-formats.png') })
    }
    check(errors.length === 0, 'no browser exceptions after finalization')
    check(unexpected.length === 0, 'finalization and copying use only fixture API requests')
    check(external.length === 0, 'final results make no external requests')
    const report = {
      passed: true,
      baseline: Boolean(baseline),
      hiddenMount,
      offline,
      assertions: checks.length,
      streamingAssertions,
      checks,
      geometry,
      errors,
      unexpected,
      external,
    }
    if (process.env.PROBE_REPORT) await fs.writeFile(process.env.PROBE_REPORT, JSON.stringify(report, null, 2))
    stage('passed ' + checks.length + ' assertions')
    console.log(JSON.stringify(report, null, 2))
  } catch (error) {
    if (page && process.env.PROBE_SCREENSHOT_DIR) {
      await fs.mkdir(process.env.PROBE_SCREENSHOT_DIR, { recursive: true })
      await page.screenshot({ path: path.join(process.env.PROBE_SCREENSHOT_DIR, 'failure.png') }).catch(() => {})
    }
    const states = page
      ? await page
          .evaluate(() =>
            Object.fromEntries(
              (window.__outputProbe?.ids || ['short', 'long', 'offscreen', 'terminal']).map((id) => {
                const value = window.__outputProbe?.state(id) || window.__outputProbe?.staticState(id)
                if (value) {
                  value.lifecycle = window.__outputProbe.lifecycle(id)
                  value.textTail = value.text.slice(-2000)
                  delete value.text
                  delete value.visibleText
                }
                return [id, value]
              }),
            ),
          )
          .catch(() => null)
      : null
    const report = {
      passed: false,
      baseline: Boolean(baseline),
      hiddenMount,
      offline,
      error: error.stack,
      checks,
      geometry,
      states,
      errors,
      unexpected,
      external,
    }
    if (process.env.PROBE_REPORT) await fs.writeFile(process.env.PROBE_REPORT, JSON.stringify(report, null, 2))
    console.error(JSON.stringify(report, null, 2))
    throw error
  } finally {
    for (const resource of resources.values()) for (const client of resource.clients) client.response.end()
    await Promise.allSettled([browser?.close(), server?.close()])
    await fs.rm(temporary, { recursive: true, force: true })
  }
}
main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
