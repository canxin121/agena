// Native Chromium regression probe. Requires a production build, Playwright,
// and Chrome/Chromium. Transport is injected through CDP; Notification and
// Service Worker APIs are real. This does not assert that an OS banner appeared.
const assert = require('node:assert/strict')
const http = require('node:http')
const fs = require('node:fs/promises')
const path = require('node:path')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright')

async function until(read, predicate, label) {
  for (let attempt = 0; attempt < 100; attempt++) {
    const value = await read()
    if (predicate(value)) return value
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`Timed out: ${label}`)
}

async function main() {
  const root = path.resolve(__dirname, '../dist')
  const server = http.createServer(async (req, res) => {
    if (req.url === '/probe') {
      res.setHeader('content-type', 'text/html')
      res.end('<!doctype html><title>Browser notification probe</title>')
      return
    }
    if (req.url !== '/agena-notifications-sw.js') {
      res.writeHead(404).end()
      return
    }
    res.setHeader('content-type', 'text/javascript')
    try {
      res.end(await fs.readFile(path.join(root, 'agena-notifications-sw.js')))
    } catch {
      res.writeHead(404).end()
    }
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  const origin = `http://127.0.0.1:${server.address().port}`
  let browser
  try {
    browser = await chromium.launch({
      executablePath: process.env.CHROME_BIN || undefined,
      headless: true,
      args: ['--no-sandbox', '--disable-dev-shm-usage'],
    })
    const context = await browser.newContext({ permissions: ['notifications'] })
    const control = await context.newPage()
    const cdp = await context.newCDPSession(control)
    const registrations = new Map()
    const versions = new Map()
    const transitions = []
    cdp.on('ServiceWorker.workerRegistrationUpdated', ({ registrations: rows }) =>
      rows.forEach((row) => registrations.set(row.registrationId, row)),
    )
    cdp.on('ServiceWorker.workerVersionUpdated', ({ versions: rows }) =>
      rows.forEach((row) => {
        versions.set(row.versionId, row)
        transitions.push(row.runningStatus)
      }),
    )
    await cdp.send('ServiceWorker.enable')
    const page = await context.newPage()
    await page.goto(`${origin}/probe`)
    await page.evaluate(async () => {
      await navigator.serviceWorker.register('/agena-notifications-sw.js', { scope: '/' })
      await navigator.serviceWorker.ready
    })
    const registration = await until(
      async () => [...registrations.values()].find((row) => row.scopeURL === `${origin}/`),
      Boolean,
      'worker registration',
    )
    let inspectPage
    async function notifications() {
      if (inspectPage)
        return inspectPage.evaluate(async () =>
          (await (await navigator.serviceWorker.getRegistration('/')).getNotifications()).map((item) => ({
            title: item.title,
            body: item.body,
            tag: item.tag,
            data: item.data,
          })),
        )
      const worker = context.serviceWorkers().find((item) => item.url().endsWith('/agena-notifications-sw.js'))
      return worker.evaluate(async () =>
        (await self.registration.getNotifications()).map((item) => ({
          title: item.title,
          body: item.body,
          tag: item.tag,
          data: item.data,
        })),
      )
    }
    async function push(event) {
      await cdp.send('ServiceWorker.deliverPushMessage', {
        origin,
        registrationId: registration.registrationId,
        data: JSON.stringify(event),
      })
    }
    const frozen = { id: 'probe-question', session_id: 7, title: 'Question', body: 'AI is waiting for an answer.' }
    await (await context.newCDPSession(page)).send('Page.setWebLifecycleState', { state: 'frozen' })
    await push(frozen)
    await until(
      notifications,
      (items) => items.some((item) => item.body === frozen.body),
      'notification with frozen page',
    )
    await page.close()
    const version = [...versions.values()].find(
      (row) => row.registrationId === registration.registrationId && row.status === 'activated',
    )
    assert.ok(version)
    await cdp.send('ServiceWorker.stopWorker', { versionId: version.versionId })
    await until(
      async () => versions.get(version.versionId)?.runningStatus,
      (state) => state === 'stopped',
      'worker stops after page closes',
    )
    const completed = { id: 'probe-completed', session_id: 8, title: 'Finished', body: 'AI finished its reply.' }
    await push(completed)
    await until(
      async () => versions.get(version.versionId)?.runningStatus,
      (state) => state === 'running',
      'push wakes stopped worker',
    )
    // This blank reader is opened after delivery; it has no notification code.
    // Playwright's old worker handle cannot evaluate after a worker restart.
    inspectPage = await context.newPage()
    await inspectPage.goto(`${origin}/probe`)
    const shown = await until(
      notifications,
      (items) => items.some((item) => item.body === completed.body),
      'notification without an open page',
    )
    assert.equal(shown.find((item) => item.body === completed.body).data.url, '/chat?sessionId=8')
    const permission = { id: 'probe-permission', session_id: 9, title: 'Approval', body: 'AI needs your approval.' }
    await push(permission)
    await until(notifications, (items) => items.some((item) => item.body === permission.body), 'approval notification')
    await push(completed)
    const retried = await until(
      notifications,
      (items) => items.length === 3,
      'duplicate replaces existing notification',
    )
    assert.equal(retried.filter((item) => item.tag === 'agena:push:probe-completed').length, 1)
    console.log(
      JSON.stringify(
        {
          browser: await browser.version(),
          nativeBrowserApis: true,
          transport: 'CDP push injection',
          frozenPage: true,
          closedPage: true,
          stoppedWorkerWokenByPush: true,
          retryDeduplicated: true,
          workerTransitions: transitions,
          notifications: retried,
        },
        null,
        2,
      ),
    )
  } finally {
    await browser?.close()
    await new Promise((resolve) => server.close(resolve))
  }
}
main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
