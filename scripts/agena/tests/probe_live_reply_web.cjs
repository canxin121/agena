// Real browser acceptance of live replies. All models/data are local TestKit fixtures.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const os = require('node:os')
const path = require('node:path')
const { spawn } = require('node:child_process')
const { pathToFileURL } = require('node:url')
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
async function wait(check, label, ms = 60000) {
  const deadline = Date.now() + ms
  while (Date.now() < deadline) { const value = await check(); if (value) return value; await sleep(50) }
  throw new Error(`Timed out: ${label}`)
}
class Cdp {
  constructor(socket) {
    this.socket = socket; this.id = 0; this.pending = new Map(); this.subscriptions = []
    socket.addEventListener('message', ({data}) => {
      const message = JSON.parse(String(data))
      if (message.method === 'Network.requestWillBeSent' && message.params.request.url.includes('/changes/stream')) this.subscriptions.push(message.params.request.url)
      const pending = this.pending.get(message.id)
      if (!pending) return
      this.pending.delete(message.id); clearTimeout(pending.timer)
      if (message.error) pending.reject(new Error(JSON.stringify(message.error))); else pending.resolve(message.result)
    })
  }
  send(method, params = {}) {
    const id = ++this.id
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`CDP timeout: ${method}`)) }, method === 'Page.navigate' ? 60000 : 20000)
      this.pending.set(id, {resolve, reject, timer}); this.socket.send(JSON.stringify({id, method, params}))
    })
  }
  async evaluate(expression) {
    const result = await this.send('Runtime.evaluate', {expression, awaitPromise:true, returnByValue:true})
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails))
    return result.result?.value
  }
}
async function main() {
  const backend = process.env.LIVE_BACKEND
  assert(['127.0.0.1','localhost','[::1]'].includes(new URL(backend).hostname))
  const cases = JSON.parse(process.env.LIVE_CASES)
  const evidence = process.env.LIVE_EVIDENCE
  const web = path.resolve(__dirname, '../../../packages/agena-web')
  const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'agena-live-browser-'))
  await fs.mkdir(evidence, {recursive:true})
  let server, chrome, cdp
  const report = []
  try {
    const { createServer } = await import(pathToFileURL(require.resolve('vite', {paths:[web]})).href)
    server = await createServer({root:web, server:{host:'127.0.0.1',port:0,proxy:{'/api':{target:backend,ws:true},'/auth':backend,'/health':backend}}})
    await server.listen()
    const base = `http://127.0.0.1:${server.httpServer.address().port}`
    chrome = spawn(process.env.CHROME_BIN || '/usr/bin/google-chrome', ['--headless=new','--no-sandbox','--disable-dev-shm-usage','--no-first-run','--disable-background-networking','--disable-component-update','--disable-sync','--disable-extensions','--remote-debugging-address=127.0.0.1','--remote-debugging-port=0',`--user-data-dir=${temporary}`,'about:blank'], {stdio:'ignore'})
    const port = await wait(async () => { try { return Number((await fs.readFile(path.join(temporary,'DevToolsActivePort'),'utf8')).split('\n')[0]) } catch { return false } }, 'Chrome startup')
    const page = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, {method:'PUT'})).json()
    const socket = new WebSocket(page.webSocketDebuggerUrl)
    await new Promise((resolve, reject) => { socket.addEventListener('open',resolve,{once:true}); socket.addEventListener('error',reject,{once:true}) })
    cdp = new Cdp(socket)
    await cdp.send('Page.enable'); await cdp.send('Runtime.enable'); await cdp.send('Network.enable')
    await cdp.send('Emulation.setDeviceMetricsOverride',{width:1280,height:900,deviceScaleFactor:1,mobile:false})
    for (const item of cases) {
      const route = item.bot_id ? `/bot?botId=${item.bot_id}&sessionId=${item.session_id}` : `/chat?sessionId=${item.session_id}`
      console.log('LIVE_WEB_NAVIGATE', item.label)
      await cdp.send('Page.navigate',{url:base+route})
      const input = `Array.from(document.querySelectorAll('[data-chat-input="true"]')).find(n=>n.getClientRects().length)`
      await wait(() => cdp.evaluate(`Boolean(${input}) && new URLSearchParams(location.search).get('sessionId') === '${item.session_id}'`), `composer for ${item.label}`)
      await wait(() => cdp.subscriptions.some((url) => { const query = new URL(url).searchParams; return query.get('scope_kind') === 'session' && query.get('session_id') === String(item.session_id) }), `scoped live tail for ${item.label}`, 8000)
      await cdp.evaluate(`(${input}).focus()`)
      await cdp.send('Input.insertText',{text:item.web_prompt})
      const started = Date.now()
      for (const type of ['keyDown','keyUp']) await cdp.send('Input.dispatchKeyEvent',{type,key:'Enter',code:'Enter',windowsVirtualKeyCode:13,modifiers:2})
      await wait(() => cdp.evaluate(`document.body.innerText.includes(${JSON.stringify(item.web_reply)})`), `visible ${item.label} reply without refresh`, 8000)
      report.push({client:'web',label:item.label,sessionId:item.session_id,submitToVisibleMs:Date.now()-started})
      const image = await cdp.send('Page.captureScreenshot',{format:'png'})
      await fs.writeFile(path.join(evidence,`${item.label}-web.png`),Buffer.from(image.data,'base64'))
      await fs.writeFile(path.join(evidence,'web-latency.json'),JSON.stringify(report,null,2))
    }
    console.log('WEB_LIVE_REPLY',JSON.stringify(report))
  } catch (error) {
    if (cdp) {
      const body = await cdp.evaluate('document.body.innerText').catch(()=>'')
      await fs.writeFile(path.join(evidence,'web-failure.txt'),body)
    }
    throw error
  } finally {
    cdp?.socket.close(); chrome?.kill('SIGTERM'); await server?.close()
    await fs.rm(temporary,{recursive:true,force:true}).catch(()=>{})
  }
}
main().catch((error)=>{ console.error(error); process.exitCode=1 })
