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
      const node = document.querySelector(${JSON.stringify(selector)});
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

async function main() {
  assert(!process.versions.bun, 'Run this Vite/CDP probe with Node, not Bun')
  const backend = process.env.ASSISTANT_BACKEND?.replace(/\/$/, '')
  const source = Number(process.env.ASSISTANT_SOURCE)
  assert(
    backend && ['127.0.0.1', 'localhost', '[::1]'].includes(new URL(backend).hostname),
    'Use an isolated local Agena fixture',
  )
  assert(Number.isSafeInteger(source) && source > 0, 'ASSISTANT_SOURCE must name the fixture conversation')
  const web = path.resolve(__dirname, '..')
  const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'agena-assistant-browser-'))
  const evidence = process.env.ASSISTANT_EVIDENCE || temporary
  await fs.mkdir(evidence, { recursive: true })
  let server, chrome, cdp
  const errors = [],
    external = []
  try {
    const { createServer } = await import('vite')
    server = await createServer({
      root: web,
      server: {
        host: '127.0.0.1',
        port: 0,
        proxy: { '/api': { target: backend, ws: true }, '/auth': backend, '/health': backend },
      },
    })
    await server.listen()
    const base = `http://127.0.0.1:${server.httpServer.address().port}`
    chrome = spawn(
      process.env.CHROME_BIN || '/usr/bin/google-chrome',
      [
        '--headless=new',
        '--no-sandbox',
        '--disable-dev-shm-usage',
        '--no-first-run',
        '--no-default-browser-check',
        '--disable-background-networking',
        '--disable-component-update',
        '--disable-sync',
        '--disable-extensions',
        '--remote-debugging-address=127.0.0.1',
        '--remote-debugging-port=0',
        `--user-data-dir=${temporary}`,
        'about:blank',
      ],
      { stdio: ['ignore', 'ignore', 'pipe'] },
    )
    let chromeErrors = ''
    chrome.stderr.on('data', (chunk) => {
      chromeErrors = (chromeErrors + chunk).slice(-20000)
    })
    const port = await wait(
      async () => Number((await fs.readFile(path.join(temporary, 'DevToolsActivePort'), 'utf8')).split('\n')[0]),
      'Chrome startup',
    )
    const page = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: 'PUT' })).json()
    const socket = new WebSocket(page.webSocketDebuggerUrl)
    await new Promise((resolve, reject) => {
      socket.addEventListener('open', resolve, { once: true })
      socket.addEventListener('error', reject, { once: true })
    })
    cdp = new Cdp(socket)
    cdp.events.set('Runtime.exceptionThrown', (value) =>
      errors.push(value.exceptionDetails.exception?.description || JSON.stringify(value)),
    )
    cdp.events.set('Fetch.requestPaused', (value) => {
      const url = new URL(value.request.url)
      const local =
        ['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname) || ['data:', 'blob:'].includes(url.protocol)
      if (!local) external.push(value.request.url)
      void cdp
        .send(local ? 'Fetch.continueRequest' : 'Fetch.failRequest', {
          requestId: value.requestId,
          ...(!local ? { errorReason: 'BlockedByClient' } : {}),
        })
        .catch((error) => errors.push(String(error)))
    })
    await cdp.send('Runtime.enable')
    await cdp.send('Page.enable')
    await cdp.send('Fetch.enable', { patterns: [{ urlPattern: '*' }] })
    await cdp.send('Emulation.setDeviceMetricsOverride', {
      width: 1360,
      height: 900,
      deviceScaleFactor: 1,
      mobile: false,
    })
    if (process.env.CHANNEL_ACCESS_PROBE === '1') {
      async function detail() {
        const list = await (await fetch(`${backend}/api/v1/sessions/${source}/integrations`)).json()
        const item = list.bindings[0]
        assert(item, 'channel binding fixture')
        return (await fetch(`${backend}/api/v1/sessions/${source}/integrations/${item.id}`)).json()
      }
      await cdp.send('Page.navigate', { url: `${base}/integrations?source=${source}` })
      await wait(
        () => cdp.evaluate(`Boolean(document.querySelector('[data-testid="channel-access-panel"]'))`),
        'channel role panel',
        90000,
      )
      await cdp.click('[data-testid="channel-access-edit"]')
      await cdp.click('[data-testid="channel-member-add"]')
      const existingOwner = await cdp.evaluate(`document.querySelector('[data-testid="channel-member-0"]').value`)
      await cdp.fill('[data-testid="channel-member-1"]', existingOwner)
      await cdp.click('[data-testid="channel-access-save"]')
      await wait(
        () => cdp.evaluate(`Boolean(document.querySelector('[role="dialog"] [role="alert"]')?.textContent)`),
        'duplicate member policy rejected',
      )
      await wait(
        () => cdp.evaluate(`!document.querySelector('[data-testid="channel-member-1"]').matches(':disabled')`),
        'invalid role proposal remains editable',
      )
      await cdp.fill('[data-testid="channel-member-1"]', 'web-member')
      await cdp.click('[role="dialog"] details > summary')
      await cdp.fill('[data-testid="channel-mcp-operator"]', '{"docs":["search"]}')
      await cdp.click('[data-testid="channel-access-save"]')
      const changed = await wait(async () => {
        const d = await detail()
        return d.binding.spec.channel_policy.members.some((m) => m.actor === 'web-member') ? d : null
      }, 'actual member policy saved')
      assert.equal(changed.binding.spec.channel_policy.members.find((m) => m.actor === 'web-member').role, 'member')
      assert.equal(changed.access_audit[0].actor, 'instance_operator')
      assert.deepEqual(changed.binding.spec.channel_policy.grants.operator.mcp_tools, { docs: ['search'] })
      await wait(() => cdp.evaluate(`!document.querySelector('[role="dialog"]')`), 'access modal closes')
      await wait(
        () =>
          cdp.evaluate(
            `document.querySelector('[data-testid="channel-access-panel"]')?.dataset.accessRevision==='${changed.binding.revision}' && !document.querySelector('[data-testid="channel-access-edit"]').disabled`,
          ),
        'saved roles rendered before re-edit',
      )
      await cdp.click('[data-testid="channel-access-edit"]')
      await wait(
        () => cdp.evaluate(`Boolean(document.querySelector('[role="dialog"] [data-testid="channel-role-1"]'))`),
        'saved member role editor mounted',
      )
      await cdp.evaluate(
        `(()=>{const e=document.querySelector('[data-testid="channel-role-1"]');e.value='blocked';e.dispatchEvent(new Event('change',{bubbles:true}));})()`,
      )
      const latest = await detail()
      const policy = latest.binding.spec.channel_policy
      policy.per_user_daily_limit = 25
      const changedElsewhere = await fetch(
        `${backend}/api/v1/sessions/${source}/integrations/${latest.binding.id}/access`,
        {
          method: 'PUT',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({
            expected_revision: latest.binding.revision,
            request_key: 'web-policy-concurrent',
            policy,
          }),
        },
      )
      assert(changedElsewhere.ok, 'concurrent policy fixture mutation')
      await cdp.click('[data-testid="channel-access-save"]')
      await wait(
        () => cdp.evaluate(`document.querySelector('[role="dialog"] [role="alert"]')?.textContent.length>0`),
        'stale role save rejected',
      )
      const preserved = await detail()
      assert.equal(preserved.binding.spec.channel_policy.members.find((m) => m.actor === 'web-member').role, 'member')
      assert.equal(preserved.binding.spec.channel_policy.per_user_daily_limit, 25)
      assert(
        await cdp.evaluate(`document.querySelector('[data-testid="channel-access-save"]').disabled`),
        'stale save must require review',
      )
      await cdp.click('[data-testid="channel-access-close"]')
      await cdp.click('[data-testid="integrations-refresh"]')
      await wait(
        () =>
          cdp.evaluate(
            `document.querySelector('[data-testid="channel-access-panel"]')?.dataset.accessRevision==='${preserved.binding.revision}'`,
          ),
        'latest access revision rendered',
      )
      await cdp.screenshot(path.join(evidence, 'channel-access-desktop.png'))
      await cdp.send('Emulation.setDeviceMetricsOverride', {
        width: 375,
        height: 812,
        deviceScaleFactor: 1,
        mobile: true,
      })
      await cdp.click('[data-testid="channel-access-edit"]')
      assert(
        await cdp.evaluate('document.documentElement.scrollWidth <= innerWidth+1'),
        'channel access mobile overflow',
      )
      await cdp.screenshot(path.join(evidence, 'channel-access-mobile.png'))
      assert.deepEqual(errors, [])
      assert.deepEqual(external, [])
      console.log(
        JSON.stringify({
          ok: true,
          source,
          checks: [
            'member roles editor',
            'invalid policy can be corrected',
            'actual policy save',
            'audit identity',
            'stale role conflict',
            'mobile access form',
          ],
          evidence,
        }),
      )
      return
    }
    if (process.env.INTEGRATIONS_PROBE === '1') {
      async function integrations() {
        const response = await fetch(`${backend}/api/v1/sessions/${source}/integrations`)
        assert(response.ok, `integration catalog ${response.status}`)
        return response.json()
      }
      await cdp.send('Page.navigate', { url: `${base}/integrations?source=${source}` })
      await wait(
        () => cdp.evaluate(`Boolean(document.querySelector('[data-provider="example.github_issues"]'))`),
        'installed external provider in Web',
        90000,
      )
      await cdp.click('[data-provider="example.github_issues"]')
      const proposal = await cdp.evaluate(
        `JSON.parse(document.querySelector('[data-testid="integration-json"]').value)`,
      )
      proposal.title = 'UI integration fixture'
      proposal.resource = 'owner/repository'
      proposal.config = { repository: 'owner/repository' }
      await cdp.fill('[data-testid="integration-json"]', JSON.stringify(proposal, null, 2))
      await cdp.click('[data-testid="integration-create"]')
      const binding = await wait(async () => (await integrations()).bindings[0], 'disabled draft persisted')
      assert.equal(binding.state, 'disabled', 'save silently enabled external authority')
      await wait(() => cdp.evaluate(`!document.querySelector('[role="dialog"]')`), 'draft dialog closes')
      await cdp.click('[data-testid="integration-enable"]')
      assert.equal((await integrations()).bindings[0].state, 'disabled', 'opening review already enabled grant')
      await cdp.click('[data-testid="integration-confirm"]')
      await wait(async () => (await integrations()).bindings[0].state === 'enabled', 'explicit review enables grant')
      await wait(() => cdp.evaluate(`!document.querySelector('[role="dialog"]')`), 'enable confirmation closes')
      await cdp.click('[data-testid="integration-pause"]')
      await cdp.click('[data-testid="integration-confirm"]')
      await wait(async () => (await integrations()).bindings[0].state === 'paused', 'pause revokes grant')
      await wait(() => cdp.evaluate(`!document.querySelector('[role="dialog"]')`), 'pause confirmation closes')
      await cdp.screenshot(path.join(evidence, 'integrations-desktop.png'))
      await cdp.send('Emulation.setDeviceMetricsOverride', {
        width: 375,
        height: 812,
        deviceScaleFactor: 1,
        mobile: true,
      })
      assert(
        await cdp.evaluate('document.documentElement.scrollWidth <= innerWidth+1'),
        'integration page overflows on mobile',
      )
      await cdp.screenshot(path.join(evidence, 'integrations-mobile.png'))
      assert.deepEqual(errors, [])
      assert.deepEqual(external, [])
      console.log(
        JSON.stringify({
          ok: true,
          source,
          binding: binding.id,
          checks: [
            'external provider discovery',
            'disabled proposal',
            'explicit enable review',
            'pause',
            'mobile layout',
          ],
          evidence,
        }),
      )
      return
    }
    if (process.env.ASSISTANT_PROBE === '1') {
      async function assistant() {
        const response = await fetch(`${backend}/api/v1/sessions/${source}/assistant`)
        assert(response.ok, `assistant snapshot ${response.status}`)
        return response.json()
      }
      async function chat(text, marker) {
        await cdp.fill('[data-chat-input="true"]', text)
        await cdp.send('Input.dispatchKeyEvent', {
          type: 'keyDown',
          key: 'Enter',
          code: 'Enter',
          windowsVirtualKeyCode: 13,
          modifiers: 2,
        })
        await cdp.send('Input.dispatchKeyEvent', {
          type: 'keyUp',
          key: 'Enter',
          code: 'Enter',
          windowsVirtualKeyCode: 13,
          modifiers: 2,
        })
        await wait(() => cdp.evaluate(`document.body.innerText.includes(${JSON.stringify(marker)})`), marker, 60000)
      }
      await cdp.send('Page.navigate', { url: `${base}/chat?sessionId=${source}` })
      await wait(
        () => cdp.evaluate(`Boolean(document.querySelector('[data-chat-input="true"]'))`),
        'assistant chat input',
        90000,
      )
      await chat('以后叫你小鱼，叫我阿林', 'assistant-ui-persona-saved')
      await chat('记住我希望报告先给结论', 'assistant-ui-memory-saved')
      await chat('两秒后提醒我核对结果', 'assistant-ui-reminder-scheduled')
      await wait(
        () => cdp.evaluate(`document.body.innerText.includes('assistant-ui-contact-delivered')`),
        'proactive reminder in same chat',
        60000,
      )
      await chat('一小时后再提醒我一次', 'assistant-ui-later-scheduled')
      let state = await assistant()
      assert.equal(state.profile.preferences.name, '小鱼')
      assert.equal(state.memories[0]?.name, 'ui-report-style')
      assert.equal(state.reminders.filter((r) => r.state === 'completed').length, 1)
      const future = state.reminders.find((r) => r.state === 'active')
      assert(future, 'future reminder')
      // The project chat header no longer exposes Assistant settings; the
      // panel is a standalone route (also reachable via the /assistant slash
      // command) and must not be entangled with project session chrome.
      await cdp.send('Page.navigate', { url: `${base}/assistant?source=${source}` })
      await wait(() => cdp.evaluate(`location.pathname==='/assistant'`), 'assistant route')
      // The panel shell renders before its asynchronous profile request settles.
      // Wait for the visible form and completed initial load before reading values.
      await wait(
        () =>
          cdp.evaluate(`(() => {
            const name = document.querySelector('[data-testid="assistant-name"]');
            const refresh = document.querySelector('[data-testid="assistant-refresh"]');
            return Boolean(name?.getClientRects().length && refresh && !refresh.disabled);
          })()`),
        'loaded assistant profile',
      )
      assert.equal(await cdp.evaluate(`document.querySelector('[data-testid="assistant-name"]').value`), '小鱼')
      // Hold a real GET snapshot from before the edit. A late refresh must
      // not roll a successful profile write back to an older revision.
      await cdp.evaluate(`(() => { const original=window.fetch; let hold=true;
        window.fetch=async(...args)=>{const response=await original(...args);
          if(hold&&String(args[0]).endsWith('/assistant')&&(!args[1]?.method||args[1].method==='GET')){
            hold=false;window.__assistantReadHeld=true;await new Promise(resolve=>window.__releaseAssistantRead=resolve);
          }return response;};})()`)
      await cdp.click('[data-testid="assistant-refresh"]')
      await wait(() => cdp.evaluate('window.__assistantReadHeld===true'), 'held pre-edit snapshot')
      await cdp.fill('[data-testid="assistant-name"]', 'Web小鱼')
      await cdp.click('[data-testid="assistant-save"]')
      await wait(async () => (await assistant()).profile.preferences.name === 'Web小鱼', 'saved UI persona')
      await cdp.evaluate('window.__releaseAssistantRead()')
      await wait(
        () => cdp.evaluate(`document.querySelector('[data-testid="assistant-name"]').value==='Web小鱼'`),
        'late read cannot undo saved profile',
      )
      // Concurrent edits must not be silently rebased by a manual refresh.
      await cdp.fill('[data-testid="assistant-address"]', '阿林-更新')
      const currentProfile = (await assistant()).profile
      const externalEdit = await fetch(`${backend}/api/v1/sessions/${source}/assistant`, {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          expected_revision: currentProfile.revision,
          request_key: 'ui-external-timezone-change',
          patch: { timezone: 'Asia/Shanghai' },
        }),
      })
      assert(externalEdit.ok, 'fixture concurrent profile edit')
      await cdp.click('[data-testid="assistant-refresh"]')
      await cdp.click('[data-testid="assistant-save"]')
      await wait(
        () => cdp.evaluate(`Boolean(document.querySelector('[data-testid="assistant-reconfirm"]'))`),
        'profile conflict requires reconfirmation',
      )
      await cdp.click('[data-testid="assistant-reconfirm"]')
      await cdp.click('[data-testid="assistant-save"]')
      await wait(
        async () => (await assistant()).profile.preferences.address_user === '阿林-更新',
        'reconciled profile edit',
      )
      assert.equal(
        (await assistant()).profile.preferences.timezone,
        'Asia/Shanghai',
        'reconfirm overwrote an unrelated newer setting',
      )
      await cdp.click('[data-testid="assistant-tab-memory"]')
      await cdp.click('[data-memory-name="ui-report-style"] [data-testid="assistant-memory-edit"]')
      await cdp.fill('[data-testid="assistant-memory-body"]', 'assistant-ui-memory-corrected: action items first')
      await cdp.click('[data-testid="assistant-memory-submit"]')
      await wait(
        async () => (await assistant()).memories.some((m) => m.body.includes('assistant-ui-memory-corrected')),
        'corrected stored memory',
      )
      await wait(() => cdp.evaluate(`!document.querySelector('[role="dialog"]')`), 'memory edit closes')
      await cdp.screenshot(path.join(evidence, 'assistant-memory-desktop.png'))
      await cdp.click('[data-memory-name="ui-report-style"] [data-testid="assistant-memory-delete"]')
      await cdp.click('[data-testid="assistant-memory-submit"]')
      await wait(async () => (await assistant()).memories.length === 0, 'actual memory deletion')
      await wait(() => cdp.evaluate(`!document.querySelector('[role="dialog"]')`), 'memory delete closes')
      await cdp.click('[data-testid="assistant-tab-reminders"]')
      await cdp.click(`[data-reminder-id="${future.id}"] [data-testid="assistant-reminder-cancel"]`)
      await cdp.click('[data-testid="assistant-reminder-confirm"]')
      await wait(
        async () => (await assistant()).reminders.some((r) => r.id === future.id && r.state === 'cancelled'),
        'cancel schedule',
      )
      await wait(() => cdp.evaluate(`!document.querySelector('[role="dialog"]')`), 'schedule confirmation closes')
      await cdp.click('[data-testid="assistant-tab-profile"]')
      await cdp.click('[data-testid="assistant-proactive-enabled"]')
      await cdp.click('[data-testid="assistant-save"]')
      await wait(async () => !(await assistant()).profile.preferences.proactive_enabled, 'mute proactive contact')
      await cdp.evaluate('window.__assistantBeforeReload = true')
      await cdp.send('Page.reload')
      await wait(
        () =>
          cdp.evaluate(
            `window.__assistantBeforeReload !== true && document.querySelector('[data-testid="assistant-name"]')?.value==='Web小鱼' && document.querySelector('[data-testid="assistant-proactive-enabled"]')?.checked === false`,
          ),
        'new document reload persists profile and mute state',
        90000,
      )
      await cdp.screenshot(path.join(evidence, 'assistant-profile-desktop.png'))
      await cdp.send('Emulation.setDeviceMetricsOverride', {
        width: 375,
        height: 812,
        deviceScaleFactor: 1,
        mobile: true,
      })
      assert(await cdp.evaluate('document.documentElement.scrollWidth <= innerWidth+1'), 'assistant mobile overflow')
      await cdp.screenshot(path.join(evidence, 'assistant-profile-mobile.png'))
      assert.deepEqual(errors, [])
      assert.deepEqual(external, [])
      console.log(
        JSON.stringify({
          ok: true,
          source,
          checks: [
            'natural persona',
            'natural memory',
            'proactive same-chat reminder',
            'profile editor',
            'memory correction/deletion',
            'cancel schedule',
            'mute persists',
            'mobile layout',
          ],
          evidence,
        }),
      )
      return
    }
    throw new Error('Choose ASSISTANT_PROBE=1, INTEGRATIONS_PROBE=1 or CHANNEL_ACCESS_PROBE=1')
  } catch (error) {
    if (cdp) {
      await fs.writeFile(path.join(evidence, 'browser-errors.json'), JSON.stringify({ errors, external }, null, 2))
      await fs.writeFile(
        path.join(evidence, 'browser-failure.txt'),
        await cdp.evaluate('document.body.innerText').catch(() => 'unavailable'),
      )
      await cdp.screenshot(path.join(evidence, 'browser-failure.png')).catch(() => {})
    }
    throw error
  } finally {
    cdp?.close()
    if (chrome && chrome.exitCode === null) {
      chrome.kill('SIGTERM')
      await Promise.race([new Promise((resolve) => chrome.once('exit', resolve)), sleep(3000)])
      if (chrome.exitCode === null) chrome.kill('SIGKILL')
    }
    await server?.close()
    if (evidence !== temporary) await fs.rm(temporary, { recursive: true, force: true })
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
