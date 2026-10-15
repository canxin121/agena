// Run against an isolated Agena API + Vite proxy. See docs/interactive-replies.md.
import assert from 'node:assert/strict'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
const { chromium } = await import(process.env.AGENA_UI_TEST_BROWSER_MODULE || 'playwright-core')
const base = process.env.AGENA_UI_TEST_URL || 'http://127.0.0.1:5179'
const directory = resolve(fileURLToPath(new URL('../../../..', import.meta.url)))
const temp = await mkdtemp(join(tmpdir(), 'agena-rich-reply-'))
const owner = `browser-${Date.now()}`
const browser = await chromium.launch({
  headless: true,
  ...(process.env.AGENA_UI_TEST_CHROME_PATH ? { executablePath: process.env.AGENA_UI_TEST_CHROME_PATH } : {}),
})
const page = await browser.newPage({ viewport: { width: 1100, height: 1000 } })
page.setDefaultTimeout(20000)
const errors = []
page.on('pageerror', (error) => errors.push(error.message))
try {
  const query = new URLSearchParams({ directory, owner, database: join(temp, 'tasks.sqlite') })
  await page.goto(`${base}/tests/fixtures/rich-replies.html?${query}`)
  await page.getByRole('button', { name: '运行应用', exact: true }).click()
  const frame = page.frameLocator('iframe')
  await frame.getByText('Connected · Changes saved in SQLite', { exact: true }).waitFor()
  const taskTitle = 'Verify a real backend mutation'
  await frame.getByRole('textbox', { name: 'New task' }).fill(taskTitle)
  await frame.getByRole('button', { name: '+ Add task', exact: true }).click()
  await frame.getByText(taskTitle, { exact: true }).waitFor()
  await frame.getByRole('checkbox', { name: `Complete ${taskTitle}`, exact: true }).check()
  await frame.locator('.task.done').filter({ hasText: taskTitle }).waitFor()
  const iframe = page.frames().find((f) => f !== page.mainFrame())
  assert.ok(iframe)
  const stateRoute = `**/api/v1/workbench/apps/${owner}/flowboard/state`
  // Force the first write to arrive late. The bridge must still preserve the
  // order of calls made without awaiting each individual UI event handler.
  await page.route(stateRoute, async (route) => {
    if (route.request().postDataJSON()?.sequence === 1) await new Promise((resolve) => setTimeout(resolve, 200))
    await route.continue()
  })
  const state = await iframe.evaluate(async () => {
    await Promise.all([
      agena.setState({ sequence: 1 }),
      agena.setState({ sequence: 2 }),
      agena.setState({ sequence: 3 }),
    ])
    return agena.getState()
  })
  assert.deepEqual(state, { sequence: 3 })
  const persisted = await (await page.request.get(`${base}/api/v1/workbench/apps/${owner}/flowboard`)).json()
  assert.deepEqual(persisted.widgetState, { sequence: 3 })
  await page.unroute(stateRoute)
  const isolation = await iframe.evaluate(async () => {
    let hostBlocked = false,
      storageBlocked = false,
      networkBlocked = false
    try {
      void parent.document.body
    } catch {
      hostBlocked = true
    }
    try {
      localStorage.getItem('test')
    } catch {
      storageBlocked = true
    }
    try {
      await fetch('/api/v1/health')
    } catch {
      networkBlocked = true
    }
    window.streamingMarker = 'preserved'
    return { hostBlocked, storageBlocked, networkBlocked }
  })
  assert.deepEqual(isolation, { hostBlocked: true, storageBlocked: true, networkBlocked: true })
  await page.evaluate(() => window.richReplyFixture.append('\n\nMore streaming text.'))
  assert.equal(await iframe.evaluate(() => window.streamingMarker), 'preserved')
  let stateWrite = page.waitForResponse((r) => r.url().endsWith('/state') && r.request().method() === 'PUT')
  await frame.getByRole('button', { name: 'Active', exact: true }).click()
  await stateWrite
  stateWrite = page.waitForResponse((r) => r.url().endsWith('/state') && r.request().method() === 'PUT')
  await frame.getByRole('searchbox', { name: 'Search tasks' }).fill('make')
  await stateWrite
  await page.getByRole('button', { name: '停止', exact: true }).click()
  await page.getByRole('button', { name: '运行应用', exact: true }).click()
  await frame.getByText('Connected · Changes saved in SQLite', { exact: true }).waitFor()
  assert.equal(await frame.getByRole('searchbox', { name: 'Search tasks' }).inputValue(), 'make')
  assert.equal(await frame.getByRole('button', { name: 'Active', exact: true }).getAttribute('aria-pressed'), 'true')
  await frame.getByRole('searchbox', { name: 'Search tasks' }).fill('')
  await frame.getByRole('button', { name: 'All', exact: true }).click()
  await frame.locator('.task.done').filter({ hasText: taskTitle }).waitFor()
  await page.getByRole('button', { name: '数据表', exact: true }).click()
  assert.equal(await page.locator('[data-reply-chart] tbody tr').count(), 7)
  for (const type of ['line', 'scatter', 'bar', 'area']) {
    await page.evaluate((type) => window.richReplyFixture.setType(type), type)
    assert.equal(await page.locator('[data-reply-chart] svg[role="img"]').count(), 1)
  }
  await page.screenshot({ path: join(temp, 'desktop.png'), fullPage: true })
  await page.setViewportSize({ width: 390, height: 844 })
  // Let responsive iframe layout and its height bridge settle before capturing.
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  await page.waitForFunction(() => {
    const heading = document.querySelector('.app-heading')?.getBoundingClientRect()
    const actions = document.querySelector('.app-actions')?.getBoundingClientRect()
    return heading && actions && actions.top >= heading.bottom
  })
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
  assert.equal(await page.locator('[data-interactive-app]').count(), 1)
  assert.equal(await iframe.evaluate(() => document.querySelectorAll('main.board').length), 1)
  await page.mouse.move(0, 0)
  await iframe.evaluate(() => scrollTo(0, 0))
  await page.evaluate(() => scrollTo(0, 0))
  await page.waitForFunction(() => scrollY === 0)
  await page.screenshot({ path: join(temp, 'mobile-viewport.png') })
  await page.screenshot({ path: join(temp, 'mobile.png'), fullPage: true })
  await page.locator('iframe').screenshot({ path: join(temp, 'mobile-app.png') })
  await page.getByRole('button', { name: 'Toggle full preview', exact: true }).click()
  await frame.getByText('Connected · Changes saved in SQLite', { exact: true }).waitFor()
  await frame.getByText(taskTitle, { exact: true }).waitFor()
  await frame.getByRole('button', { name: `Delete ${taskTitle}`, exact: true }).click()
  await frame.getByText(taskTitle, { exact: true }).waitFor({ state: 'hidden' })
  await page.getByRole('button', { name: '停止', exact: true }).click()
  assert.deepEqual(errors, [])
  console.log(
    `Rich reply lifecycle, real CRUD, state, streaming identity, sandbox, charts, mobile and full preview passed. Screenshots: ${temp}`,
  )
} catch (error) {
  await page.screenshot({ path: join(temp, 'failure.png'), fullPage: true }).catch(() => {})
  console.error(`Failure screenshot: ${join(temp, 'failure.png')}`)
  throw error
} finally {
  await page.request.post(`${base}/api/v1/workbench/apps/${owner}/flowboard/stop`).catch(() => {})
  await browser.close()
  // Retain the screenshots while removing the test database.
  await rm(join(temp, 'tasks.sqlite'), { force: true })
}
