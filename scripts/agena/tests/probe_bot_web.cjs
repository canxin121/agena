// Real Vue application + real Agena HTTP API. Native Chrome DevTools protocol
// avoids installing a second browser/testing stack. No external model/network.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const os = require('node:os')
const path = require('node:path')
const { spawn } = require('node:child_process')

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
async function wait(check, description, ms = 45000) {
  const until = Date.now() + ms
  let last
  while (Date.now() < until) {
    try {
      const value = await check()
      if (value) return value
    } catch (error) {
      last = error
    }
    await sleep(100)
  }
  throw new Error(`Timed out: ${description}${last ? ` (${last})` : ''}`)
}

class Cdp {
  constructor(socket) {
    this.socket = socket
    this.sequence = 0
    this.pending = new Map()
    this.events = new Map()
    socket.addEventListener('message', (event) => {
      const message = JSON.parse(String(event.data))
      if (message.id) {
        const pending = this.pending.get(message.id)
        if (!pending) return
        clearTimeout(pending.timer)
        this.pending.delete(message.id)
        if (message.error) pending.reject(new Error(JSON.stringify(message.error)))
        else pending.resolve(message.result)
      } else this.events.get(message.method)?.(message.params)
    })
  }
  send(method, params = {}) {
    const id = ++this.sequence
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`CDP timeout: ${method}`))
      }, 30000)
      this.pending.set(id, { resolve, reject, timer })
      this.socket.send(JSON.stringify({ id, method, params }))
    })
  }
  async evaluate(expression) {
    const reply = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true })
    if (reply.exceptionDetails)
      throw new Error(reply.exceptionDetails.exception?.description || JSON.stringify(reply.exceptionDetails))
    return reply.result?.value
  }
  async click(selector) {
    const point = await wait(
      () =>
        this.evaluate(`(() => {
      const node = Array.from(document.querySelectorAll(${JSON.stringify(selector)})).find(node => node.getClientRects().length);
      if (!node || node.disabled || !node.getClientRects().length) return null;
      node.scrollIntoView({block:'center'}); const r = node.getBoundingClientRect();
      return {x:r.x+r.width/2,y:r.y+r.height/2}; })()`),
      selector,
    )
    await this.send('Input.dispatchMouseEvent', { type: 'mousePressed', ...point, button: 'left', clickCount: 1 })
    await this.send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...point, button: 'left', clickCount: 1 })
  }
  async fill(selector, text) {
    await this.click(selector)
    await this.send('Input.dispatchKeyEvent', {
      type: 'keyDown',
      key: 'a',
      code: 'KeyA',
      windowsVirtualKeyCode: 65,
      modifiers: 2,
    })
    await this.send('Input.dispatchKeyEvent', {
      type: 'keyUp',
      key: 'a',
      code: 'KeyA',
      windowsVirtualKeyCode: 65,
      modifiers: 2,
    })
    await this.send('Input.insertText', { text })
  }
  async key(key, code = key, number = 0) {
    await this.send('Input.dispatchKeyEvent', { type: 'keyDown', key, code, windowsVirtualKeyCode: number })
    await this.send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode: number })
  }
  async screenshot(file) {
    const { data } = await this.send('Page.captureScreenshot', { format: 'png' })
    await fs.writeFile(file, Buffer.from(data, 'base64'))
  }
  close() {
    for (const pending of this.pending.values()) clearTimeout(pending.timer)
    this.socket.close()
  }
}


// Real browser acceptance for the canonical operator Bot; isolated localhost only.
async function main() {
  assert(!process.versions.bun, 'Use Node for the Vite/CDP probe')
  const backend = process.env.BOT_BACKEND?.replace(/\/$/, '')
  assert(backend && ['127.0.0.1', 'localhost', '[::1]'].includes(new URL(backend).hostname))
  const source = Number(process.env.BOT_PROJECT_SESSION)
  assert(Number.isSafeInteger(source) && source > 0)
  const web = path.resolve(__dirname, '../../../packages/agena-web')
  const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'agena-bot-browser-'))
  const evidence = process.env.BOT_EVIDENCE || temporary
  await fs.mkdir(evidence, {recursive:true})
  const errors = [], external = [], sends = []
  let server, chrome, cdp
  const input = '[data-chat-input="true"]'
  async function ready() {
    return wait(() => cdp.evaluate(`Array.from(document.querySelectorAll(${JSON.stringify(input)})).some(node => node.getClientRects().length)`), 'visible full chat composer', 90000)
  }
  async function bodyHas(text) {
    return cdp.evaluate(`document.body.innerText.includes(${JSON.stringify(text)})`)
  }
  async function send(text) {
    await ready()
    await cdp.fill(input, text)
    for (const type of ['keyDown','keyUp']) await cdp.send('Input.dispatchKeyEvent', {type, key:'Enter',code:'Enter',windowsVirtualKeyCode:13,modifiers:2})
  }
  async function canonical() {
    const r = await fetch(`${backend}/api/v1/bot/session`, {method:'POST'})
    assert(r.ok, `ensure bot: ${r.status}`)
    const value = await r.json()
    assert(Number.isSafeInteger(value.session_id) && value.session_id > 0)
    return value.session_id
  }
  try {
    const {pathToFileURL} = require('node:url')
    const {createServer} = await import(pathToFileURL(require.resolve('vite', {paths:[web]})).href)
    server = await createServer({root:web, server:{host:'127.0.0.1', port:0, proxy:{'/api':{target:backend,ws:true},'/auth':backend,'/health':backend}}})
    await server.listen()
    const base = `http://127.0.0.1:${server.httpServer.address().port}`
    chrome = spawn(process.env.CHROME_BIN || '/usr/bin/google-chrome', [
      '--headless=new','--no-sandbox','--disable-dev-shm-usage','--no-first-run','--no-default-browser-check',
      '--disable-background-networking','--disable-component-update','--disable-sync','--disable-extensions',
      '--remote-debugging-address=127.0.0.1','--remote-debugging-port=0',`--user-data-dir=${temporary}`,'about:blank',
    ], {stdio:['ignore','ignore','pipe']})
    chrome.stderr.on('data', () => {})
    const port = await wait(async () => Number((await fs.readFile(path.join(temporary,'DevToolsActivePort'),'utf8')).split('\n')[0]), 'Chrome startup')
    const page = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, {method:'PUT'})).json()
    const socket = new WebSocket(page.webSocketDebuggerUrl)
    await new Promise((resolve,reject) => {socket.addEventListener('open',resolve,{once:true}); socket.addEventListener('error',reject,{once:true})})
    cdp = new Cdp(socket)
    cdp.events.set('Runtime.exceptionThrown', value => errors.push(value.exceptionDetails.exception?.description || JSON.stringify(value)))
    cdp.events.set('Fetch.requestPaused', value => {
      const url = new URL(value.request.url)
      const local = ['127.0.0.1','localhost','[::1]'].includes(url.hostname) || ['data:','blob:'].includes(url.protocol)
      if (!local) external.push(value.request.url)
      if (value.request.method === 'POST' && /\/api\/v1\/sessions\/\d+\/messages$/.test(url.pathname)) sends.push({path:url.pathname,body:value.request.postData || ''})
      void cdp.send(local ? 'Fetch.continueRequest' : 'Fetch.failRequest', {requestId:value.requestId,...(!local ? {errorReason:'BlockedByClient'}:{})}).catch(e=>errors.push(String(e)))
    })
    await cdp.send('Runtime.enable'); await cdp.send('Page.enable')
    await cdp.send('Fetch.enable',{patterns:[{urlPattern:'*'}]})
    await cdp.send('Emulation.setDeviceMetricsOverride',{width:1360,height:900,deviceScaleFactor:1,mobile:false})

    // A direct entry works without first selecting a project. The query must not
    // be able to substitute an ordinary project conversation for the Bot.
    await cdp.send('Page.navigate',{url:`${base}/bot?sessionId=${source}`})
    await ready()
    const bot = await canonical()
    assert.notEqual(bot, source)
    await wait(() => cdp.evaluate(`location.pathname === '/bot' && new URLSearchParams(location.search).get('sessionId') === '${bot}'`), 'server canonical bot identity')
    assert(await bodyHas('AI Bot'), 'Bot identity is visible')
    assert(await cdp.evaluate(`Boolean(document.querySelector('[data-main-nav-tab="bot"]')?.getClientRects().length)`), 'desktop Bot entry')
    assert(await cdp.evaluate(`(() => {
      const composer = Array.from(document.querySelectorAll('[data-chat-input="true"]')).find(node => node.getClientRects().length)?.getBoundingClientRect();
      if (!composer) return false;
      return Array.from(document.querySelectorAll('[data-sonner-toast]')).every(node => {
        const r = node.getBoundingClientRect();
        return r.bottom <= composer.top || r.top >= composer.bottom || r.right <= composer.left || r.left >= composer.right;
      });
    })()`), 'notification offers do not cover the Bot composer')
    await cdp.screenshot(path.join(evidence,'bot-web-welcome-desktop.png'))
    await send('bot-ui-web-message')
    await wait(() => bodyHas('bot-ui-web-reply'), 'real Bot response', 60000)
    assert(sends.some(x=>x.path===`/api/v1/sessions/${bot}/messages` && x.body.includes('bot-ui-web-message')), 'Bot message went to canonical session')
    if (process.env.BOT_EXPECT_ORGANIZED === '1') {
      const response = await fetch(`${backend}/api/v1/sessions/${source}`)
      assert(response.ok, 'native session remains openable after Bot organization')
      const organized = await response.json()
      assert(organized.favorite === true && organized.pinned === true, 'real Bot tool updated favorite/pinned in the shared native session')
    }
    await cdp.fill(input,'bot-ui-unsent-draft')

    // Project transcript is still the native human-facing conversation.
    await cdp.evaluate('window.__botProbeOldDocument = true')
    await cdp.send('Page.navigate',{url:`${base}/chat?sessionId=${source}`})
    await wait(() => cdp.evaluate(`window.__botProbeOldDocument !== true && location.pathname === '/chat' && new URLSearchParams(location.search).get('sessionId') === '${source}'`), 'explicit project deep link in a new document', 90000)
    await ready()
    assert(!await bodyHas('bot-ui-web-reply'), 'new project view does not reuse outgoing Bot transcript')
    await send('bot-ui-project-message')
    await wait(() => bodyHas('bot-ui-project-reply'), 'ordinary project response', 60000)
    assert(sends.some(x=>x.path===`/api/v1/sessions/${source}/messages` && x.body.includes('bot-ui-project-message')), 'project message stayed in project')
    assert(!sends.some(x=>x.path===`/api/v1/sessions/${bot}/messages` && x.body.includes('bot-ui-project-message')), 'no project message sent to Bot')
    await cdp.click('[data-main-nav-tab="bot"]')
    await ready()
    await wait(() => bodyHas('bot-ui-web-reply'), 'Bot history on navigation')
    assert.equal(await canonical(), bot, 'no duplicate Bot')
    assert(!await bodyHas('bot-ui-project-reply'), 'project response not leaking into Bot')
    await cdp.screenshot(path.join(evidence,'bot-web-chat-desktop.png'))

    // Repeated navigation and document reload must preserve server identity.
    await cdp.click('[data-main-nav-tab="bot"]')
    await ready()
    await cdp.evaluate('window.__botProbeDocument = "old"')
    await cdp.send('Page.reload')
    await wait(() => cdp.evaluate('window.__botProbeDocument !== "old"'), 'new document')
    await ready()
    await wait(() => bodyHas('bot-ui-web-reply'), 'Bot history after reload')
    assert.equal(await canonical(), bot)

    await cdp.send('Emulation.setDeviceMetricsOverride',{width:375,height:812,deviceScaleFactor:1,mobile:true})
    await wait(() => cdp.evaluate(`Boolean(document.querySelector('.oc-bottom-nav a[href="/bot"]')?.getClientRects().length)`), 'mobile Bot navigation')
    assert(await cdp.evaluate('document.documentElement.scrollWidth <= innerWidth + 1'), 'no horizontal mobile overflow')
    // Switching desktop pane hosting to the mobile surface can remount the
    // canonical conversation. Wait for that real visible composer, not just
    // the navigation shell, before measuring the responsive input.
    await ready()
    await wait(() => cdp.evaluate(`(() => {const node=Array.from(document.querySelectorAll(${JSON.stringify(input)})).find(node => node.getClientRects().length);if(!node)return false;const r=node.getBoundingClientRect();return r.width>120&&r.x>=0&&r.right<=innerWidth+1;})()`), 'usable mobile composer')
    await cdp.screenshot(path.join(evidence,'bot-web-chat-mobile.png'))
    await cdp.click('.oc-bottom-nav a[href="/bot"]')
    await ready()
    assert.equal(await canonical(), bot)
    assert.deepEqual(errors,[], 'no browser runtime exceptions')
    assert.deepEqual(external,[], 'no external network requests')
    const result={ok:true,bot,project_session:source,checks:['direct entry','forged query rejected','desktop navigation','real Bot chat','native favorite/pinned management','project continuation','correct send targets','history','repeat entry','reload','375px mobile'],evidence}
    await fs.writeFile(path.join(evidence,'web-result.json'), JSON.stringify(result,null,2))
    console.log(JSON.stringify(result))
  } catch(error) {
    if(cdp) {try {await cdp.screenshot(path.join(evidence,'bot-web-failure.png')); await fs.writeFile(path.join(evidence,'bot-web-failure.txt'),String(await cdp.evaluate('document.body.innerText')))}catch{}}
    await fs.writeFile(path.join(evidence,'browser-errors.json'), JSON.stringify({errors,external,sends},null,2))
    throw error
  } finally {
    cdp?.close(); chrome?.kill('SIGTERM'); await server?.close()
    await fs.rm(temporary,{recursive:true,force:true,maxRetries:4,retryDelay:150})
  }
}
main().catch(error=>{console.error(error);process.exitCode=1})
