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
      const x=r.x+r.width/2,y=r.y+r.height/2; return node.contains(document.elementFromPoint(x,y)) ? {x,y,touch:matchMedia('(pointer:coarse)').matches} : null; })()`),
      selector,
    )
    if (point.touch) {
      await this.send('Input.dispatchTouchEvent', {type:'touchStart',touchPoints:[{x:point.x,y:point.y}]})
      await this.send('Input.dispatchTouchEvent', {type:'touchEnd',touchPoints:[]})
    } else {
      for (const type of ['mousePressed','mouseReleased']) await this.send('Input.dispatchMouseEvent', {type,x:point.x,y:point.y,button:'left',clickCount:1})
    }
  }
  async clickText(selector, label) {
    const point = await wait(() => this.evaluate(`(() => {
      const n=[...document.querySelectorAll(${JSON.stringify(selector)})].find(el=>el.getClientRects().length && !el.disabled && el.textContent.includes(${JSON.stringify(label)}));
      if(!n) return null; n.scrollIntoView({block:'center'}); const r=n.getBoundingClientRect();
      const x=r.x+r.width/2,y=r.y+r.height/2; return n.contains(document.elementFromPoint(x,y)) ? {x,y,touch:matchMedia('(pointer:coarse)').matches} : null;
    })()`), `visible ${selector} containing ${label}`)
    if(point.touch) {
      await this.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x:point.x,y:point.y}]})
      await this.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]})
    } else {
      for(const type of ['mousePressed','mouseReleased']) await this.send('Input.dispatchMouseEvent',{type,x:point.x,y:point.y,button:'left',clickCount:1})
    }
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


// Real multi-profile acceptance. All HTTP responses come from the fixture
// service; interception records sends and blocks external network only.
async function main() {
  assert(!process.versions.bun, 'Use Node for the Vite/CDP probe')
  const backend = process.env.BOT_BACKEND?.replace(/\/$/, '')
  assert(backend && ['127.0.0.1', 'localhost', '[::1]'].includes(new URL(backend).hostname))
  const a = JSON.parse(process.env.BOT_PROFILE_A)
  const b = JSON.parse(process.env.BOT_PROFILE_B)
  assert(a.id !== b.id && a.session_id !== b.session_id)
  const source = Number(process.env.BOT_PROJECT_SESSION)
  const web = path.resolve(__dirname, '../../../packages/agena-web')
  const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'agena-multi-bot-browser-'))
  const evidence = process.env.BOT_EVIDENCE || temporary
  await fs.mkdir(evidence, {recursive:true})
  const errors = [], external = [], sends = [], profileRequests = [], uploads = [], responses = []
  let server, chrome, cdp
  const input = '[data-chat-input="true"]'
  const test = name => `[data-testid="${name}"]`
  async function api(route) {
    const r = await fetch(`${backend}${route}`)
    assert(r.ok, `${route}: HTTP ${r.status} ${await r.clone().text()}`)
    return r.json()
  }
  const visible = selector => `Array.from(document.querySelectorAll(${JSON.stringify(selector)})).find(n => n.getClientRects().length)`
  async function assertGlobal(profile) {
    const resource = await api(`/api/v1/sessions/${profile.session_id}`)
    const state = await api(`/api/v1/sessions/${profile.session_id}/state`)
    assert.equal(resource.workspace_id, null, 'real Bot session association is null')
    assert.equal(state.session.workspace_id, null, 'real execution session association is null')
    assert.equal(state.execution.effective_workspace_root ?? null, null, 'real execution root is null')
  }
  async function assertBotSidebar() {
    await wait(() => cdp.evaluate(`Boolean(${visible(test('bot-sidebar'))})`), 'standalone Bot sidebar')
    const sidebar = await cdp.evaluate(`(() => { const n=${visible(test('bot-sidebar'))}; return {text:n.innerText, projectRows:n.querySelectorAll('[data-keyboard-item^="directory:"], [data-keyboard-item^="session:"]').length}; })()`)
    assert.equal(sidebar.projectRows, 0, 'Bot sidebar has no project directory or session rows')
    for (const text of ['Project sidebar sentinel 732', 'Favorites', 'Recent', process.env.BOT_PROJECT_PATH])
      assert(!sidebar.text.includes(text), `Bot sidebar leaked project section ${text}`)
    assert(!await bodyHas(process.env.BOT_PROJECT_PATH), 'Bot header/composer exposes project cwd')
  }
  async function ready(profile) {
    await wait(() => cdp.evaluate(`Boolean(${visible(input)})`), 'visible Bot composer', 90000)
    await wait(() => cdp.evaluate(`new URLSearchParams(location.search).get('botId') === '${profile.id}' && new URLSearchParams(location.search).get('sessionId') === '${profile.session_id}'`), `canonical route for ${profile.name}`)
    await wait(() => cdp.evaluate(`Array.from(document.querySelectorAll('[data-testid="bot-name"]')).some(n=>n.getClientRects().length && n.textContent.includes(${JSON.stringify(profile.name)}))`), 'selected name matches bound session')
  }
  const bodyHas = text => cdp.evaluate(`document.body.innerText.includes(${JSON.stringify(text)})`)
  const draft = () => cdp.evaluate(`(() => {const n=${visible(input)};return n?.value ?? n?.innerText ?? '';})()`)
  async function message(text, profile, reply, prefilled = false) {
    await ready(profile)
    if (!prefilled) await cdp.fill(input,text)
    for (const type of ['keyDown','keyUp']) await cdp.send('Input.dispatchKeyEvent',{type,key:'Enter',code:'Enter',windowsVirtualKeyCode:13,modifiers:2})
    await wait(() => bodyHas(reply),`real response ${reply}`,60000)
    const requests=sends.filter(x=>x.body.includes(text))
    assert.equal(requests.length,1,`one dispatch for ${text}`)
    assert.equal(requests[0].path,`/api/v1/sessions/${profile.session_id}/messages`,'correct selected Bot target')
  }
  // The standalone Bot sidebar is the selector: its rows are the same list
  // primitives the project session sidebar uses.
  async function choose(profile) {
    await cdp.click(`[data-sidebar-bot-id="${profile.id}"] [data-oc-list-item-primary]`)
    await ready(profile)
  }
  async function navigate(profile, forged = '') {
    await cdp.evaluate('window.__multiBotOld = true')
    await cdp.send('Page.navigate',{url:`${base}/bot?botId=${profile.id}${forged}`})
    await wait(() => cdp.evaluate('window.__multiBotOld !== true'),'new browser document',90000)
    await ready(profile)
  }
  let base
  try {
    const {pathToFileURL}=require('node:url')
    const {createServer}=await import(pathToFileURL(require.resolve('vite',{paths:[web]})).href)
    server=await createServer({root:web,server:{host:'127.0.0.1',port:0,proxy:{'/api':{target:backend,ws:true},'/auth':backend,'/health':backend}}})
    await server.listen()
    base=`http://127.0.0.1:${server.httpServer.address().port}`
    chrome=spawn(process.env.CHROME_BIN || '/usr/bin/google-chrome',[
      '--headless=new','--lang=en-US','--no-sandbox','--disable-dev-shm-usage','--no-first-run','--no-default-browser-check',
      '--disable-background-networking','--disable-component-update','--disable-sync','--disable-extensions',
      '--remote-debugging-address=127.0.0.1','--remote-debugging-port=0',`--user-data-dir=${temporary}`,'about:blank',
    ],{stdio:['ignore','ignore','pipe']})
    chrome.stderr.on('data',()=>{})
    const port=await wait(async()=>Number((await fs.readFile(path.join(temporary,'DevToolsActivePort'),'utf8')).split('\n')[0]),'Chrome startup')
    const page=await(await fetch(`http://127.0.0.1:${port}/json/new?about:blank`,{method:'PUT'})).json()
    const socket=new WebSocket(page.webSocketDebuggerUrl)
    await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true})})
    cdp=new Cdp(socket)
    const failedRequests = new Map()
    cdp.events.set('Network.responseReceived', value => {
      if (value.response.url.includes('/api/v1/')) responses.push({url:value.response.url,status:value.response.status})
      if (value.response.status >= 400 && value.response.url.includes('/api/v1/')) failedRequests.set(value.requestId, value.response.url)
    })
    cdp.events.set('Network.loadingFinished', value => {
      const url = failedRequests.get(value.requestId)
      if (url) void cdp.send('Network.getResponseBody', {requestId:value.requestId}).then(body=>responses.push({url,body:body.body})).catch(()=>{})
    })
    cdp.events.set('Runtime.exceptionThrown',value=>errors.push(value.exceptionDetails.exception?.description || JSON.stringify(value)))
    cdp.events.set('Fetch.requestPaused',value=>{
      const url=new URL(value.request.url)
      const local=['127.0.0.1','localhost','[::1]'].includes(url.hostname) || ['data:','blob:'].includes(url.protocol)
      if(!local)external.push(value.request.url)
      if (/^\/api\/v1\/bots(?:\/|$)/.test(url.pathname)) profileRequests.push({method:value.request.method,path:url.pathname,body:value.request.postData ? JSON.parse(value.request.postData) : null})
      if(value.request.method==='POST' && /\/api\/v1\/sessions\/\d+\/attachments$/.test(url.pathname)) uploads.push(url.pathname)
      if(value.request.method==='POST' && /\/api\/v1\/sessions\/\d+\/messages$/.test(url.pathname))sends.push({path:url.pathname,body:value.request.postData || ''})
      void cdp.send(local?'Fetch.continueRequest':'Fetch.failRequest',{requestId:value.requestId,...(!local?{errorReason:'BlockedByClient'}:{})}).catch(e=>errors.push(String(e)))
    })
    await cdp.send('Runtime.enable');await cdp.send('Page.enable');await cdp.send('Network.enable')
    await cdp.send('Fetch.enable',{patterns:[{urlPattern:'*'}]})
    await cdp.send('Emulation.setDeviceMetricsOverride',{width:1360,height:900,deviceScaleFactor:1,mobile:false})
    // Registry management must work without putting Bots into project chrome.
    await cdp.send('Page.navigate',{url:`${base}/chat?sessionId=${source}`})
    await wait(()=>cdp.evaluate(`location.pathname==='/chat' && Boolean(${visible(test('bot-sidebar'))})===false`),'project sidebar has no Bot section',120000)
    await wait(()=>cdp.evaluate(`!document.querySelector('[data-sidebar-bot-id]')`),'project sidebar has no Bot rows')
    assert(await cdp.evaluate(`Boolean(document.querySelector('.oc-chat-sidebar')?.getClientRects().length)`),'project session sidebar is visible')
    assert(!await cdp.evaluate(`Boolean(${visible(test('chat-assistant'))})`),'project chat header hides assistant settings')
    const projectRoute=await cdp.evaluate('location.pathname+location.search')
    const aBefore=await api(`/api/v1/bots/${a.id}`)
    await navigate(a)
    await wait(()=>cdp.evaluate(`Boolean(${visible(test('bot-sidebar'))})`),'standalone Bot sidebar')
    // Row edit goes through the shared overflow menu and the Bot manager.
    await cdp.click(`[data-sidebar-bot-id="${b.id}"] ${test('sidebar-bot-actions')}`)
    await cdp.clickText('[data-option-menu-item]', 'Edit Bot')
    await cdp.fill(test('bot-edit-instructions'),'Manager edited B personality persisted')
    await cdp.click(test('bot-save'))
    await wait(async()=>(await api(`/api/v1/bots/${b.id}`)).persona.instructions==='Manager edited B personality persisted','Bot edit persisted')
    assert.deepEqual(await api(`/api/v1/bots/${a.id}`),aBefore,'editing B leaves A unchanged')
    await cdp.click(test('sidebar-bot-create'))
    await cdp.fill(test('bot-edit-name'),'Browser created profile')
    await cdp.click(test('bot-save'))
    const c=await wait(async()=>(await api('/api/v1/bots')).items.find(p=>p.name==='Browser created profile'),'sidebar-created durable Bot')
    await ready(c)
    await cdp.click(`[data-sidebar-bot-id="${c.id}"] ${test('sidebar-bot-actions')}`)
    await cdp.clickText('[data-option-menu-item]', 'Archive Bot')
    await wait(()=>cdp.evaluate(`Boolean(${visible(test('bot-archive-confirm'))})`),'archive confirmation')
    assert.equal((await api(`/api/v1/bots/${c.id}`)).archived,false,'no archive before confirmation')
    await cdp.click(test('bot-archive-confirm'))
    await wait(async()=>(await api(`/api/v1/bots/${c.id}`)).archived,'archive retained registry record')
    assert(profileRequests.filter(r=>r.method!=='GET' && !r.path.endsWith('/session')).every(r=>r.body?.request_key),'profile mutations include idempotency keys')
    assert(profileRequests.filter(r=>r.method==='PUT' || r.path.endsWith('/archive')).every(r=>Number.isInteger(r.body?.expected_revision)),'profile updates use revision CAS')
    await navigate(a,`&sessionId=${source}`)
    await assertGlobal(a); await assertGlobal(b); await assertBotSidebar()
    await cdp.send('Page.navigate',{url:`${base}/chat?sessionId=${source}`})
    await wait(()=>cdp.evaluate(`location.pathname==='/chat' && Boolean(document.querySelector('.oc-chat-sidebar')?.getClientRects().length) && !Boolean(document.querySelector('[data-sidebar-bot-id]'))`),'explicit project link restores project UI')
    assert.equal(projectRoute.startsWith('/chat'),true,'registry work never leaked into project route')
    const projectResource=await api(`/api/v1/sessions/${source}`)
    assert(Number.isInteger(projectResource.workspace_id) && projectResource.workspace_id > 0, 'project link retains project association')
    await navigate(a)
    await assertBotSidebar()
    await cdp.fill(input,'multi-web-a-message')
    const imageFile=path.join(temporary,'global-ui-image.png')
    await fs.writeFile(imageFile,Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAIAAAAmkwkpAAAAFElEQVR4nGOMalrFAANMDEgANwcATOABjhcuxq0AAAAASUVORK5CYII=','base64'))
    const documentNode=await cdp.send('DOM.getDocument')
    const fileInput=await cdp.send('DOM.querySelector',{nodeId:documentNode.root.nodeId,selector:'input[type="file"][multiple]'})
    assert(fileInput.nodeId,'real composer file input exists')
    await cdp.send('DOM.setFileInputFiles',{nodeId:fileInput.nodeId,files:[imageFile]})
    await wait(()=>bodyHas('global-ui-image.png'),'image staged in Bot composer')
    await message('multi-web-a-message',a,'multi-web-a-reply',true)
    assert(uploads.includes(`/api/v1/sessions/${a.session_id}/attachments`),'global image used owning session upload')
    await wait(()=>cdp.evaluate(`Array.from(document.images).some(n=>n.getClientRects().length && n.complete && n.naturalWidth > 0)`),'global attachment rendered')
    await cdp.screenshot(path.join(evidence,'multi-bot-web-attachment.png'))
    await cdp.fill(input,'multi-a-unsent-draft')
    await choose(b)
    assert(!await bodyHas('multi-web-a-reply'),'A transcript is absent from B')
    assert(!String(await draft()).includes('multi-a-unsent-draft'),'A draft is absent from B')
    // Set B's native per-conversation model through the real composer picker.
    await cdp.click('[aria-label="Model"]')
    await cdp.clickText('[data-option-menu-item]', 'multi-bot-model')
    await message('multi-web-b-message',b,'multi-web-b-reply')
    await cdp.fill(input,'multi-b-unsent-draft')
    await choose(a)
    await wait(()=>bodyHas('multi-web-a-reply'),'A history after picker switch')
    assert(!await bodyHas('multi-web-b-reply'),'B transcript is absent from A')
    await wait(async()=>String(await draft()).includes('multi-a-unsent-draft'),'independent A draft restored')
    const aSnapshot=await api(`/api/v1/sessions/${a.session_id}/state`)
    const bSnapshot=await api(`/api/v1/sessions/${b.session_id}/state`)
    assert.equal(aSnapshot.execution.model_id,'fake-model','A keeps its model')
    assert.equal(bSnapshot.execution.model_id,'multi-bot-model','B keeps its model')
    await cdp.click(test('bot-settings'))
    for (const removed of ['bot-edit-address', 'bot-edit-tone', 'bot-edit-language']) {
      assert(!await cdp.evaluate(`Boolean(${visible(test(removed))})`), `removed persona control ${removed} is visible`)
    }
    await cdp.fill(test('bot-edit-name'),'Aurora Web edited')
    await cdp.fill(test('bot-edit-instructions'),'Web edited A personality persisted')
    await cdp.screenshot(path.join(evidence,'multi-bot-web-persona-wide.png'))
    await cdp.click(test('bot-save'))
    await wait(async()=>{const p=await api(`/api/v1/bots/${a.id}`);return p.name==='Aurora Web edited' && p.persona.instructions==='Web edited A personality persisted'},'saved A profile from real API')
    a.name='Aurora Web edited'
    await wait(()=>cdp.evaluate(`!Boolean(${visible('[role="dialog"]')})`),'saved editor and overlay close')
    await ready(a)
    assert.equal((await api(`/api/v1/bots/${b.id}`)).name,b.name,'A editor did not rename B')
    await cdp.screenshot(path.join(evidence,'multi-bot-web-chat-wide.png'))
    await choose(b)
    await wait(async()=>String(await draft()).includes('multi-b-unsent-draft'),'independent B draft restored')
    // Explicit deep link wins browser last selection and ignores forged session.
    await navigate(b,`&sessionId=${a.session_id}`)
    await wait(()=>bodyHas('multi-web-b-reply'),'B history from bookmarked identity')
    assert(!await bodyHas('multi-web-a-reply'))
    await cdp.send('Emulation.setDeviceMetricsOverride',{width:375,height:812,deviceScaleFactor:1,mobile:true})
    await cdp.send('Emulation.setTouchEmulationEnabled',{enabled:true,maxTouchPoints:1})
    await ready(b)
    await assertGlobal(b)
    await cdp.click('[aria-label="Open sessions"]')
    await assertBotSidebar()
    await cdp.screenshot(path.join(evidence,'multi-bot-web-sidebar-375.png'))
    await cdp.click('[aria-label="Back"]')
    await ready(b)
    assert(await cdp.evaluate('document.documentElement.scrollWidth <= innerWidth + 1'),'375px viewport has no horizontal overflow')
    await cdp.screenshot(path.join(evidence,'multi-bot-web-chat-375.png'))
    // The Bot sidebar is the picker on both viewports.
    await cdp.click('[aria-label="Open sessions"]')
    await wait(()=>cdp.evaluate(`Boolean(${visible(test('bot-sidebar'))})`),'mobile Bot sidebar picker')
    await cdp.screenshot(path.join(evidence,'multi-bot-web-picker-375.png'))
    await cdp.click(`[data-sidebar-bot-id="${a.id}"] [data-oc-list-item-primary]`)
    await ready(a)
    await cdp.click(test('bot-settings'))
    await wait(() => cdp.evaluate(`(() => {const n=${visible(test('bot-edit-name'))}; return (n?.matches('input,textarea') ? n : n?.querySelector('input,textarea'))?.value === ${JSON.stringify(a.name)};})()`),'mobile editor persisted name')
    await wait(() => cdp.evaluate(`(${visible(test('bot-edit-instructions'))})?.value === 'Web edited A personality persisted'`),'mobile editor persisted instructions')
    await cdp.screenshot(path.join(evidence,'multi-bot-web-persona-375.png'))
    // Invalid explicit ids never silently expose another profile's composer.
    const bad='00000000-0000-0000-0000-000000000042'
    await cdp.send('Page.navigate',{url:`${base}/bot?botId=${bad}&sessionId=${a.session_id}`})
    await wait(()=>cdp.evaluate(`location.search.includes('${bad}') && !Boolean(${visible(input)}) && Boolean(${visible('[data-testid="bot-sidebar"]')}) && Boolean(${visible('[role="alert"]')})`),'invalid Bot explicit create/choose state')
    await cdp.screenshot(path.join(evidence,'multi-bot-web-invalid-375.png'))
    assert.deepEqual(errors,[],'no runtime exceptions')
    assert.deepEqual(external,[],'no external network requests')
    const result={ok:true,a,b,created:c.id,checks:['named picker','real A/B replies','correct outgoing targets','independent drafts/history','per-conversation model','edit persona','sidebar create/edit/archive on real registry','botId deep link','forged sessionId','invalid identity','1360px and 375px layouts'],profileRequests,evidence}
    await fs.writeFile(path.join(evidence,'multi-web-result.json'),JSON.stringify(result,null,2))
    console.log(JSON.stringify(result))
  } catch(error) {
    if(cdp){try{await cdp.screenshot(path.join(evidence,'multi-web-failure.png'));await fs.writeFile(path.join(evidence,'multi-web-failure.txt'),String(await cdp.evaluate(`JSON.stringify({url:location.href,body:document.body.innerText,dialogs:[...document.querySelectorAll('[role=dialog]')].map(n=>({text:n.innerText,visible:!!n.getClientRects().length})),active:document.activeElement?.outerHTML})`)))}catch{}}
    await fs.writeFile(path.join(evidence,'multi-browser-errors.json'),JSON.stringify({errors,external,sends,profileRequests,uploads,responses},null,2))
    throw error
  } finally {
    cdp?.close();chrome?.kill('SIGTERM');await server?.close()
    await fs.rm(temporary,{recursive:true,force:true,maxRetries:4,retryDelay:150})
  }
}
main().catch(error=>{console.error(error);process.exitCode=1})
