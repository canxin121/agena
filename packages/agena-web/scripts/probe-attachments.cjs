/** Actual Vue attachment component + Chromium, with deterministic media API responses.
 * PLAYWRIGHT_MODULE can name an existing playwright-core installation; no production server is used.
 */
const fs = require('node:fs')
const path = require('node:path')
const assert = require('node:assert/strict')
const { spawn } = require('node:child_process')
const net = require('node:net')
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright-core')
const root = path.resolve(__dirname, '..')
const output = path.resolve(root, '../../target/attachment-audit/browser')
const png = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4/5/hPwAH/QL+ppTFtAAAAABJRU5ErkJggg==',
  'base64',
)
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
async function freePort() {
  const server = net.createServer()
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  const port = server.address().port
  await new Promise((resolve) => server.close(resolve))
  return port
}
;(async () => {
  fs.mkdirSync(output, { recursive: true })
  const port = await freePort()
  const viteLog = fs.openSync(path.join(output, 'vite.log'), 'w')
  const vite = spawn(
    process.execPath,
    [
      './node_modules/vite/bin/vite.js',
      '--config',
      'tests/fixtures/vite.attachments.config.mjs',
      '--host',
      '127.0.0.1',
      '--port',
      String(port),
      '--strictPort',
    ],
    { cwd: root, stdio: ['ignore', viteLog, viteLog] },
  )
  let context
  try {
    let ready = false
    for (let i = 0; i < 100; i++) {
      try {
        ready = (
          await fetch(`http://127.0.0.1:${port}/tests/fixtures/attachment-preview.html`, {
            signal: AbortSignal.timeout(1500),
          })
        ).ok
      } catch {}
      if (ready) break
      if (vite.exitCode !== null) throw new Error(`Vite exited: ${vite.exitCode}`)
      await delay(150)
    }
    assert.ok(ready, 'isolated Vite fixture must start')
    context = await chromium.launchPersistentContext(path.join(output, 'profile'), {
      executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
      headless: true,
      args: ['--no-sandbox'],
      acceptDownloads: true,
      downloadsPath: path.join(output, 'downloads'),
    })
    const page = await context.newPage()
    const errors = []
    page.on('pageerror', (error) => errors.push(error.message))
    await page.goto(`http://127.0.0.1:${port}/tests/fixtures/attachment-preview.html`)
    await page.waitForFunction(() => {
      const images = [...document.querySelectorAll('img')]
      return images.length === 2 && images.every((image) => image.complete && image.naturalWidth === 1)
    })
    await page.locator('[data-case=generated] img').click()
    const opened = await page.evaluate(() => window.attachmentOpened)
    assert.ok(opened[0].url.startsWith('/api/v1/sessions/43/media?'))
    assert.ok(await page.locator('[data-case=generated] img').evaluate((image) => image.src.startsWith('blob:')))
    await page.waitForFunction(() =>
      ['audio', 'video'].every((kind) => {
        const media = document.querySelector(`[data-case=${kind}] ${kind}`)
        return media && media.src.startsWith('blob:') && media.readyState >= 1 && !media.error
      }),
    )
    const downloadEvent = page.waitForEvent('download')
    await page.locator('[data-case=immutable] [data-attachment-download]').click()
    const download = await downloadEvent
    assert.equal(download.suggestedFilename(), '向日葵.png')
    const destination = path.join(output, 'downloaded.png')
    await download.saveAs(destination)
    assert.deepEqual(fs.readFileSync(destination), png)
    assert.equal(await page.locator('[data-case=unavailable] img').count(), 0)
    assert.equal(await page.locator('[data-case=unavailable] [data-attachment-download]').count(), 0)
    const fileDownloadEvent = page.waitForEvent('download')
    await page.locator('[data-case=file] [data-attachment-download]').click()
    const fileDownload = await fileDownloadEvent
    assert.equal(fileDownload.suggestedFilename(), 'report.txt')
    const fileDestination = path.join(output, 'report.txt')
    await fileDownload.saveAs(fileDestination)
    assert.equal(fs.readFileSync(fileDestination, 'utf8'), 'fixture report\n')
    const popupEvent = page.waitForEvent('popup')
    await page.locator('[data-case=file] button.part-link').first().click()
    const popup = await popupEvent
    await popup.waitForURL('blob:**')
    await popup.waitForFunction(() => document.body.textContent.includes('fixture report'))
    await popup.close()
    const activeDownloadEvent = page.waitForEvent('download')
    await page.locator('[data-case=active] button.part-link').first().click()
    const activeDownload = await activeDownloadEvent
    assert.equal(activeDownload.suggestedFilename(), 'active.html')
    const unsafeExecutions = await (await fetch(`http://127.0.0.1:${port}/probe/executions`)).json()
    assert.equal(unsafeExecutions, 0, 'active attachment must download without executing in the app origin')
    const mediaRequests = await (
      await fetch(`http://127.0.0.1:${port}/probe/requests`, { signal: AbortSignal.timeout(3000) })
    ).json()
    assert.ok(!mediaRequests.some((request) => request.path.includes('not-available')))
    assert.ok(mediaRequests.some((request) => request.path.startsWith('agena:media:') && request.download === 'true'))
    assert.ok(
      mediaRequests.every((request) => request.authenticated && !request.url.includes('attachment-fixture-token')),
    )
    for (const extension of ['.wav', '.webm', '.txt'])
      assert.ok(mediaRequests.some((request) => request.path.endsWith(extension)))
    assert.deepEqual(errors, [])
    await page.screenshot({ path: path.join(output, 'attachments.png'), fullPage: true })
    const report = { passed: true, assertions: 21, mediaRequests, consoleErrors: errors, screenshot: 'attachments.png' }
    fs.writeFileSync(path.join(output, 'report.json'), JSON.stringify(report, null, 2))
    console.log(JSON.stringify(report, null, 2))
  } finally {
    if (context) await context.close()
    vite.kill('SIGTERM')
    await Promise.race([new Promise((resolve) => vite.once('exit', resolve)), delay(5000)])
    if (vite.exitCode === null) vite.kill('SIGKILL')
    fs.closeSync(viteLog)
  }
})().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
